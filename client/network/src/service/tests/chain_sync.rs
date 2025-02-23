// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

use crate::{
	config,
	service::tests::{TestNetworkBuilder, BLOCK_ANNOUNCE_PROTO_NAME},
};

use futures::prelude::*;
use libp2p::PeerId;
use sc_block_builder::BlockBuilderProvider;
use sc_client_api::HeaderBackend;
use sc_consensus::JustificationSyncLink;
use sc_network_common::{
	config::{MultiaddrWithPeerId, ProtocolId, SetConfig},
	protocol::{event::Event, role::Roles, ProtocolName},
	service::NetworkSyncForkRequest,
	sync::{SyncState, SyncStatus},
};
use sc_network_sync::{mock::MockChainSync, service::mock::MockChainSyncInterface, ChainSync};
use sp_core::H256;
use sp_runtime::traits::{Block as BlockT, Header as _};
use std::{
	sync::{Arc, RwLock},
	task::Poll,
	time::Duration,
};
use substrate_test_runtime_client::{TestClientBuilder, TestClientBuilderExt as _};

fn set_default_expecations_no_peers(
	chain_sync: &mut MockChainSync<substrate_test_runtime_client::runtime::Block>,
) {
	chain_sync.expect_poll().returning(|_| Poll::Pending);
	chain_sync.expect_status().returning(|| SyncStatus {
		state: SyncState::Idle,
		best_seen_block: None,
		num_peers: 0u32,
		queued_blocks: 0u32,
		state_sync: None,
		warp_sync: None,
	});
}

#[tokio::test]
async fn normal_network_poll_no_peers() {
	// build `ChainSync` and set default expectations for it
	let mut chain_sync =
		Box::new(MockChainSync::<substrate_test_runtime_client::runtime::Block>::new());
	set_default_expecations_no_peers(&mut chain_sync);

	// build `ChainSyncInterface` provider and set no expecations for it (i.e., it cannot be
	// called)
	let chain_sync_service =
		Box::new(MockChainSyncInterface::<substrate_test_runtime_client::runtime::Block>::new());

	let mut network = TestNetworkBuilder::new()
		.with_chain_sync((chain_sync, chain_sync_service))
		.build();

	// perform one action on network
	let _ = network.network().next_action().await;
}

#[tokio::test]
async fn request_justification() {
	let hash = H256::random();
	let number = 1337u64;

	// build `ChainSyncInterface` provider and and expect
	// `JustificationSyncLink::request_justification() to be called once
	let mut chain_sync_service =
		Box::new(MockChainSyncInterface::<substrate_test_runtime_client::runtime::Block>::new());

	chain_sync_service
		.expect_justification_sync_link_request_justification()
		.withf(move |in_hash, in_number| &hash == in_hash && &number == in_number)
		.once()
		.returning(|_, _| ());

	// build `ChainSync` and set default expecations for it
	let mut chain_sync = MockChainSync::<substrate_test_runtime_client::runtime::Block>::new();

	set_default_expecations_no_peers(&mut chain_sync);
	let mut network = TestNetworkBuilder::new()
		.with_chain_sync((Box::new(chain_sync), chain_sync_service))
		.build();

	// send "request justifiction" message and poll the network
	network.service().request_justification(&hash, number);

	// perform one action on network
	let _ = network.network().next_action().await;
}

#[tokio::test]
async fn clear_justification_requests() {
	// build `ChainSyncInterface` provider and expect
	// `JustificationSyncLink::clear_justification_requests()` to be called
	let mut chain_sync_service =
		Box::new(MockChainSyncInterface::<substrate_test_runtime_client::runtime::Block>::new());

	chain_sync_service
		.expect_justification_sync_link_clear_justification_requests()
		.once()
		.returning(|| ());

	// build `ChainSync` and set default expecations for it
	let mut chain_sync =
		Box::new(MockChainSync::<substrate_test_runtime_client::runtime::Block>::new());

	set_default_expecations_no_peers(&mut chain_sync);
	let mut network = TestNetworkBuilder::new()
		.with_chain_sync((chain_sync, chain_sync_service))
		.build();

	// send "request justifiction" message and poll the network
	network.service().clear_justification_requests();

	// perform one action on network
	let _ = network.network().next_action().await;
}

#[tokio::test]
async fn set_sync_fork_request() {
	// build `ChainSync` and set default expectations for it
	let mut chain_sync =
		Box::new(MockChainSync::<substrate_test_runtime_client::runtime::Block>::new());
	set_default_expecations_no_peers(&mut chain_sync);

	// build `ChainSyncInterface` provider and verify that the `set_sync_fork_request()`
	// call is delegated to `ChainSyncInterface` (which eventually forwards it to `ChainSync`)
	let mut chain_sync_service =
		MockChainSyncInterface::<substrate_test_runtime_client::runtime::Block>::new();

	let hash = H256::random();
	let number = 1337u64;
	let peers = (0..3).map(|_| PeerId::random()).collect::<Vec<_>>();
	let copy_peers = peers.clone();

	chain_sync_service
		.expect_set_sync_fork_request()
		.withf(move |in_peers, in_hash, in_number| {
			&peers == in_peers && &hash == in_hash && &number == in_number
		})
		.once()
		.returning(|_, _, _| ());

	let mut network = TestNetworkBuilder::new()
		.with_chain_sync((chain_sync, Box::new(chain_sync_service)))
		.build();

	// send "set sync fork request" message and poll the network
	network.service().set_sync_fork_request(copy_peers, hash, number);

	// perform one action on network
	let _ = network.network().next_action().await;
}

#[tokio::test]
async fn on_block_finalized() {
	let client = Arc::new(TestClientBuilder::with_default_backend().build_with_longest_chain().0);
	// build `ChainSyncInterface` provider and set no expecations for it (i.e., it cannot be
	// called)
	let chain_sync_service =
		Box::new(MockChainSyncInterface::<substrate_test_runtime_client::runtime::Block>::new());

	// build `ChainSync` and verify that call to `on_block_finalized()` is made
	let mut chain_sync =
		Box::new(MockChainSync::<substrate_test_runtime_client::runtime::Block>::new());

	let at = client.header(client.info().best_hash).unwrap().unwrap().hash();
	let block = client
		.new_block_at(at, Default::default(), false)
		.unwrap()
		.build()
		.unwrap()
		.block;
	let header = block.header.clone();
	let block_number = *header.number();
	let hash = block.hash();

	chain_sync
		.expect_on_block_finalized()
		.withf(move |in_hash, in_number| &hash == in_hash && &block_number == in_number)
		.once()
		.returning(|_, _| ());

	set_default_expecations_no_peers(&mut chain_sync);
	let mut network = TestNetworkBuilder::new()
		.with_client(client)
		.with_chain_sync((chain_sync, chain_sync_service))
		.build();

	// send "set sync fork request" message and poll the network
	network.network().on_block_finalized(hash, header);

	// perform one action on network
	let _ = network.network().next_action().await;
}

// report from mock import queue that importing a justification was not successful
// and verify that connection to the peer is closed
#[tokio::test]
async fn invalid_justification_imported() {
	struct DummyImportQueueHandle;

	impl
		sc_consensus::import_queue::ImportQueueService<
			substrate_test_runtime_client::runtime::Block,
		> for DummyImportQueueHandle
	{
		fn import_blocks(
			&mut self,
			_origin: sp_consensus::BlockOrigin,
			_blocks: Vec<
				sc_consensus::IncomingBlock<substrate_test_runtime_client::runtime::Block>,
			>,
		) {
		}

		fn import_justifications(
			&mut self,
			_who: sc_consensus::import_queue::RuntimeOrigin,
			_hash: substrate_test_runtime_client::runtime::Hash,
			_number: sp_runtime::traits::NumberFor<substrate_test_runtime_client::runtime::Block>,
			_justifications: sp_runtime::Justifications,
		) {
		}
	}

	struct DummyImportQueue(
		Arc<
			RwLock<
				Option<(
					PeerId,
					substrate_test_runtime_client::runtime::Hash,
					sp_runtime::traits::NumberFor<substrate_test_runtime_client::runtime::Block>,
				)>,
			>,
		>,
		DummyImportQueueHandle,
	);

	#[async_trait::async_trait]
	impl sc_consensus::ImportQueue<substrate_test_runtime_client::runtime::Block> for DummyImportQueue {
		fn poll_actions(
			&mut self,
			_cx: &mut futures::task::Context,
			link: &mut dyn sc_consensus::Link<substrate_test_runtime_client::runtime::Block>,
		) {
			if let Some((peer, hash, number)) = *self.0.read().unwrap() {
				link.justification_imported(peer, &hash, number, false);
			}
		}

		fn service(
			&self,
		) -> Box<
			dyn sc_consensus::import_queue::ImportQueueService<
				substrate_test_runtime_client::runtime::Block,
			>,
		> {
			Box::new(DummyImportQueueHandle {})
		}

		fn service_ref(
			&mut self,
		) -> &mut dyn sc_consensus::import_queue::ImportQueueService<
			substrate_test_runtime_client::runtime::Block,
		> {
			&mut self.1
		}

		async fn run(
			self,
			_link: Box<dyn sc_consensus::Link<substrate_test_runtime_client::runtime::Block>>,
		) {
		}
	}

	let justification_info = Arc::new(RwLock::new(None));
	let listen_addr = config::build_multiaddr![Memory(rand::random::<u64>())];

	let (service1, mut event_stream1) = TestNetworkBuilder::new()
		.with_import_queue(Box::new(DummyImportQueue(
			justification_info.clone(),
			DummyImportQueueHandle {},
		)))
		.with_listen_addresses(vec![listen_addr.clone()])
		.build()
		.start_network();

	let (service2, mut event_stream2) = TestNetworkBuilder::new()
		.with_set_config(SetConfig {
			reserved_nodes: vec![MultiaddrWithPeerId {
				multiaddr: listen_addr,
				peer_id: service1.local_peer_id,
			}],
			..Default::default()
		})
		.build()
		.start_network();

	async fn wait_for_events(stream: &mut (impl Stream<Item = Event> + std::marker::Unpin)) {
		let mut notif_received = false;
		let mut sync_received = false;
		while !notif_received || !sync_received {
			match stream.next().await.unwrap() {
				Event::NotificationStreamOpened { .. } => notif_received = true,
				Event::SyncConnected { .. } => sync_received = true,
				_ => {},
			};
		}
	}

	wait_for_events(&mut event_stream1).await;
	wait_for_events(&mut event_stream2).await;

	{
		let mut info = justification_info.write().unwrap();
		*info = Some((service2.local_peer_id, H256::random(), 1337u64));
	}

	let wait_disconnection = async {
		while !std::matches!(event_stream1.next().await, Some(Event::SyncDisconnected { .. })) {}
	};

	if tokio::time::timeout(Duration::from_secs(5), wait_disconnection).await.is_err() {
		panic!("did not receive disconnection event in time");
	}
}

#[tokio::test]
async fn disconnect_peer_using_chain_sync_handle() {
	let client = Arc::new(TestClientBuilder::with_default_backend().build_with_longest_chain().0);
	let listen_addr = config::build_multiaddr![Memory(rand::random::<u64>())];

	let import_queue = Box::new(sc_consensus::import_queue::mock::MockImportQueueHandle::new());
	let (chain_sync_network_provider, chain_sync_network_handle) =
		sc_network_sync::service::network::NetworkServiceProvider::new();
	let handle_clone = chain_sync_network_handle.clone();

	let (chain_sync, chain_sync_service, _) = ChainSync::new(
		sc_network_common::sync::SyncMode::Full,
		client.clone(),
		ProtocolId::from("test-protocol-name"),
		&Some(String::from("test-fork-id")),
		Roles::from(&config::Role::Full),
		Box::new(sp_consensus::block_validation::DefaultBlockAnnounceValidator),
		1u32,
		None,
		None,
		chain_sync_network_handle.clone(),
		import_queue,
		ProtocolName::from("block-request"),
		ProtocolName::from("state-request"),
		None,
	)
	.unwrap();

	let (node1, mut event_stream1) = TestNetworkBuilder::new()
		.with_listen_addresses(vec![listen_addr.clone()])
		.with_chain_sync((Box::new(chain_sync), Box::new(chain_sync_service)))
		.with_chain_sync_network((chain_sync_network_provider, chain_sync_network_handle))
		.with_client(client.clone())
		.build()
		.start_network();

	let (node2, mut event_stream2) = TestNetworkBuilder::new()
		.with_set_config(SetConfig {
			reserved_nodes: vec![MultiaddrWithPeerId {
				multiaddr: listen_addr,
				peer_id: node1.local_peer_id,
			}],
			..Default::default()
		})
		.with_client(client.clone())
		.build()
		.start_network();

	async fn wait_for_events(stream: &mut (impl Stream<Item = Event> + std::marker::Unpin)) {
		let mut notif_received = false;
		let mut sync_received = false;
		while !notif_received || !sync_received {
			match stream.next().await.unwrap() {
				Event::NotificationStreamOpened { .. } => notif_received = true,
				Event::SyncConnected { .. } => sync_received = true,
				_ => {},
			};
		}
	}

	wait_for_events(&mut event_stream1).await;
	wait_for_events(&mut event_stream2).await;

	handle_clone.disconnect_peer(node2.local_peer_id, BLOCK_ANNOUNCE_PROTO_NAME.into());

	let wait_disconnection = async {
		while !std::matches!(event_stream1.next().await, Some(Event::SyncDisconnected { .. })) {}
	};

	if tokio::time::timeout(Duration::from_secs(5), wait_disconnection).await.is_err() {
		panic!("did not receive disconnection event in time");
	}
}
