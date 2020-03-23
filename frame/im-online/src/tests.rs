// Copyright 2019-2020 Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

//! Tests for the im-online module.

#![cfg(test)]

use super::*;
use crate::mock::*;
use sp_core::offchain::{
	OpaquePeerId,
	OffchainExt,
	TransactionPoolExt,
	testing::{TestOffchainExt, TestTransactionPoolExt},
};
use frame_support::{dispatch, assert_noop};
use sp_runtime::testing::UintAuthorityId;

#[test]
fn test_unresponsiveness_slash_fraction() {
	// A single case of unresponsiveness is not slashed.
	assert_eq!(
		UnresponsivenessOffence::<()>::slash_fraction(1, 50),
		Perbill::zero(),
	);

	assert_eq!(
		UnresponsivenessOffence::<()>::slash_fraction(5, 50),
		Perbill::zero(), // 0%
	);

	assert_eq!(
		UnresponsivenessOffence::<()>::slash_fraction(7, 50),
		Perbill::from_parts(4200000), // 0.42%
	);

	// One third offline should be punished around 5%.
	assert_eq!(
		UnresponsivenessOffence::<()>::slash_fraction(17, 50),
		Perbill::from_parts(46200000), // 4.62%
	);
}

#[test]
fn should_report_offline_validators() {
	new_test_ext().execute_with(|| {
		// given
		let block = 1;
		System::set_block_number(block);
		// buffer new validators
		Session::rotate_session();
		// enact the change and buffer another one
		let validators = vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
			UintAuthorityId(4),
			UintAuthorityId(5),
			UintAuthorityId(6),
		];
		VALIDATORS.with(|l| *l.borrow_mut() = Some(validators.clone()));
		Session::rotate_session();

		// when
		// we end current session and start the next one
		Session::rotate_session();

		// then
		let offences = OFFENCES.with(|l| l.replace(vec![]));
		assert_eq!(offences, vec![
			(vec![], UnresponsivenessOffence {
				session_index: 2,
				validator_set_count: 3,
				offenders: vec![
					(UintAuthorityId(1), UintAuthorityId(1)),
					(UintAuthorityId(2), UintAuthorityId(2)),
					(UintAuthorityId(3), UintAuthorityId(3)),
				],
			})
		]);

		// should not report when heartbeat is sent
		for (idx, v) in validators.into_iter().take(4).enumerate() {
			let _ = heartbeat(block, 3, idx as u32, v.into()).unwrap();
		}
		Session::rotate_session();

		// then
		let offences = OFFENCES.with(|l| l.replace(vec![]));
		assert_eq!(offences, vec![
			(vec![], UnresponsivenessOffence {
				session_index: 3,
				validator_set_count: 6,
				offenders: vec![
					(UintAuthorityId(5), UintAuthorityId(5)),
					(UintAuthorityId(6), UintAuthorityId(6)),
				],
			})
		]);
	});
}

fn heartbeat(
	block_number: u64,
	session_index: u32,
	authority_index: u32,
	id: UintAuthorityId,
) -> dispatch::DispatchResult {
	use frame_support::unsigned::ValidateUnsigned;

	let heartbeat = Heartbeat {
		block_number,
		network_state: OpaqueNetworkState {
			peer_id: OpaquePeerId(vec![1]),
			external_addresses: vec![],
		},
		session_index,
		authority_index,
	};
	let signature = id.sign(&heartbeat.encode()).unwrap();

	ImOnline::pre_dispatch(&crate::Call::heartbeat(heartbeat.clone(), signature.clone()))
		.map_err(|e| <&'static str>::from(e))?;
	ImOnline::heartbeat(
		Origin::system(frame_system::RawOrigin::None),
		heartbeat,
		signature
	)
}

#[test]
fn should_mark_online_validator_when_heartbeat_is_received() {
	new_test_ext().execute_with(|| {
		advance_session();
		// given
		let validators = vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
			UintAuthorityId(4),
			UintAuthorityId(5),
			UintAuthorityId(6),
		];
		VALIDATORS.with(|l| *l.borrow_mut() = Some(validators.clone()));
		assert_eq!(Session::validators(), Vec::<UintAuthorityId>::new());
		// enact the change and buffer another one
		advance_session();

		assert_eq!(Session::current_index(), 2);
		assert_eq!(Session::validators(), vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
		]);

		assert!(!ImOnline::is_online(0));
		assert!(!ImOnline::is_online(1));
		assert!(!ImOnline::is_online(2));

		// when
		let _ = heartbeat(1, 2, 0, 1.into()).unwrap();

		// then
		assert!(ImOnline::is_online(0));
		assert!(!ImOnline::is_online(1));
		assert!(!ImOnline::is_online(2));

		// and when
		let _ = heartbeat(1, 2, 2, 3.into()).unwrap();

		// then
		assert!(ImOnline::is_online(0));
		assert!(!ImOnline::is_online(1));
		assert!(ImOnline::is_online(2));
	});
}

#[test]
fn late_heartbeat_should_fail() {
	new_test_ext().execute_with(|| {
		advance_session();
		// given
		let validators = vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
			UintAuthorityId(4),
			UintAuthorityId(5),
			UintAuthorityId(6),
		];
		VALIDATORS.with(|l| *l.borrow_mut() = Some(validators.clone()));
		assert_eq!(Session::validators(), Vec::<UintAuthorityId>::new());
		// enact the change and buffer another one
		advance_session();

		assert_eq!(Session::current_index(), 2);
		assert_eq!(Session::validators(), vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
		]);

		// when
		assert_noop!(heartbeat(1, 3, 0, 1.into()), "Transaction is outdated");
		assert_noop!(heartbeat(1, 1, 0, 1.into()), "Transaction is outdated");
	});
}

#[test]
fn should_generate_heartbeats() {
	use sp_runtime::traits::OffchainWorker;

	let mut ext = new_test_ext();
	let (offchain, _state) = TestOffchainExt::new();
	let (pool, state) = TestTransactionPoolExt::new();
	ext.register_extension(OffchainExt::new(offchain));
	ext.register_extension(TransactionPoolExt::new(pool));

	ext.execute_with(|| {
		// given
		let block = 1;
		System::set_block_number(block);
		UintAuthorityId::set_all_keys(vec![0, 1, 2]);
		// buffer new validators
		Session::rotate_session();
		// enact the change and buffer another one
		let validators = vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
			UintAuthorityId(4),
			UintAuthorityId(5),
			UintAuthorityId(6),
		];
		VALIDATORS.with(|l| *l.borrow_mut() = Some(validators.clone()));
		Session::rotate_session();

		// when
		ImOnline::offchain_worker(block);

		// then
		let transaction = state.write().transactions.pop().unwrap();
		// All validators have `0` as their session key, so we generate 2 transactions.
		assert_eq!(state.read().transactions.len(), 2);

		// check stuff about the transaction.
		let ex: Extrinsic = Decode::decode(&mut &*transaction).unwrap();
		let heartbeat = match ex.call {
			crate::mock::Call::ImOnline(crate::Call::heartbeat(h, _)) => h,
			e => panic!("Unexpected call: {:?}", e),
		};

		assert_eq!(heartbeat, Heartbeat {
			block_number: block,
			network_state: sp_io::offchain::network_state().unwrap(),
			session_index: 2,
			authority_index: 2,
		});
	});
}

#[test]
fn should_cleanup_received_heartbeats_on_session_end() {
	new_test_ext().execute_with(|| {
		advance_session();

		let validators = vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
		];
		VALIDATORS.with(|l| *l.borrow_mut() = Some(validators.clone()));
		assert_eq!(Session::validators(), Vec::<UintAuthorityId>::new());

		// enact the change and buffer another one
		advance_session();

		assert_eq!(Session::current_index(), 2);
		assert_eq!(Session::validators(), vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
		]);

		// send an heartbeat from authority id 0 at session 2
		let _ = heartbeat(1, 2, 0, 1.into()).unwrap();

		// the heartbeat is stored
		assert!(!ImOnline::received_heartbeats(&2, &0).is_none());

		advance_session();

		// after the session has ended we have already processed the heartbeat
		// message, so any messages received on the previous session should have
		// been pruned.
		assert!(ImOnline::received_heartbeats(&2, &0).is_none());
	});
}

#[test]
fn should_mark_online_validator_when_block_is_authored() {
	use pallet_authorship::EventHandler;

	new_test_ext().execute_with(|| {
		advance_session();
		// given
		let validators = vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
			UintAuthorityId(4),
			UintAuthorityId(5),
			UintAuthorityId(6),
		];
		VALIDATORS.with(|l| *l.borrow_mut() = Some(validators.clone()));
		assert_eq!(Session::validators(), Vec::<UintAuthorityId>::new());
		// enact the change and buffer another one
		advance_session();

		assert_eq!(Session::current_index(), 2);
		assert_eq!(Session::validators(), vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
		]);

		for i in 0..3 {
			assert!(!ImOnline::is_online(i));
		}

		// when
		ImOnline::note_author(UintAuthorityId(1));
		ImOnline::note_uncle(UintAuthorityId(2), 0);

		// then
		assert!(ImOnline::is_online(0));
		assert!(ImOnline::is_online(1));
		assert!(!ImOnline::is_online(2));
	});
}

#[test]
fn should_not_send_a_report_if_already_online() {
	use pallet_authorship::EventHandler;

	let mut ext = new_test_ext();
	let (offchain, _state) = TestOffchainExt::new();
	let (pool, pool_state) = TestTransactionPoolExt::new();
	ext.register_extension(OffchainExt::new(offchain));
	ext.register_extension(TransactionPoolExt::new(pool));

	ext.execute_with(|| {
		advance_session();
		// given
		let validators = vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
			UintAuthorityId(4),
			UintAuthorityId(5),
			UintAuthorityId(6),
		];
		VALIDATORS.with(|l| *l.borrow_mut() = Some(validators.clone()));
		assert_eq!(Session::validators(), Vec::<UintAuthorityId>::new());
		// enact the change and buffer another one
		advance_session();
		assert_eq!(Session::current_index(), 2);
		assert_eq!(Session::validators(), vec![
			UintAuthorityId(1),
			UintAuthorityId(2),
			UintAuthorityId(3),
		]);
		ImOnline::note_author(UintAuthorityId(2));
		ImOnline::note_uncle(UintAuthorityId(3), 0);

		// when
		UintAuthorityId::set_all_keys(vec![0]); // all authorities use pallet_session key 0
		// we expect error, since the authority is already online.
		let mut res = ImOnline::send_heartbeats(4).unwrap();
		assert_eq!(res.next().unwrap().unwrap(), ());
		assert_eq!(res.next().unwrap().unwrap_err(), OffchainErr::AlreadyOnline(1));
		assert_eq!(res.next().unwrap().unwrap_err(), OffchainErr::AlreadyOnline(2));
		assert_eq!(res.next(), None);

		// then
		let transaction = pool_state.write().transactions.pop().unwrap();
		// All validators have `0` as their session key, but we should only produce 1 heartbeat.
		assert_eq!(pool_state.read().transactions.len(), 0);
		// check stuff about the transaction.
		let ex: Extrinsic = Decode::decode(&mut &*transaction).unwrap();
		let heartbeat = match ex.call {
			crate::mock::Call::ImOnline(crate::Call::heartbeat(h, _)) => h,
			e => panic!("Unexpected call: {:?}", e),
		};

		assert_eq!(heartbeat, Heartbeat {
			block_number: 4,
			network_state: sp_io::offchain::network_state().unwrap(),
			session_index: 2,
			authority_index: 0,
		});
	});
}
