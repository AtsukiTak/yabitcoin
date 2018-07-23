use bitcoin::network::{message_blockdata::{GetHeadersMessage, InvType, Inventory},
                       serialize::BitcoinHash};
use bitcoin::util::hash::Sha256dHash;
use bitcoin::blockdata::block::Block;

use std::sync::mpsc::{SyncSender, TrySendError};

use connection::{Connection, OutgoingMessage};
use blockchain::{BlockChain, BlockChainMut};

pub struct Node
{
    blockchain: BlockChainMut,
    subscribers: Vec<SyncSender<BlockChain>>,
}

pub enum ProcessResult
{
    Ack,
    Ban,
}

impl Node
{
    /// Create a new `Node` with genesis block.
    pub fn new() -> Node
    {
        Node {
            blockchain: BlockChainMut::new(),
            subscribers: Vec::new(),
        }
    }

    /// Create a new `Node` with `start_block`.
    pub fn with_start(start_block: Block) -> Node
    {
        Node {
            blockchain: BlockChainMut::with_start(start_block),
            subscribers: Vec::new(),
        }
    }

    /// Add a new subscriber.
    /// Every time when underlying blockchain is updated, you get updated blockchain's snapshot.
    pub fn add_subscriber(&mut self, subscriber: SyncSender<BlockChain>)
    {
        self.subscribers.push(subscriber);
    }

    /// Send `getheaders` message to given `peer`.
    /// When we start, we need to send `getblocks` message first and then,
    /// we receive `inv` message as response.
    /// See `process::initial_block_download` for more detail.
    pub fn request_headers(&self, peer: &mut Connection) -> ProcessResult
    {
        let locator_hashes = self.blockchain.locator_blocks().map(|b| b.bitcoin_hash()).collect();
        let get_headers_msg = GetHeadersMessage::new(locator_hashes, Sha256dHash::default());
        let network_msg = OutgoingMessage::GetHeaders(get_headers_msg);
        if peer.send_msg(network_msg).is_ok() {
            ProcessResult::Ack
        } else {
            ProcessResult::Ban
        }
    }

    /// Process incoming `inv` message.
    /// `inv` message often be sent as response of `getblocks` message.
    /// After we receive `inv` message, we send `getdata` message.
    pub fn recv_inv(&self, invs: Vec<Inventory>, peer: &mut Connection) -> ProcessResult
    {
        // Check received invs all are valid.
        if !check_invs(invs.as_slice(), &self.blockchain) {
            warn!("Peer {:?} send us unwanted inventory. So we disconnect.", peer);
            return ProcessResult::Ban;
        }

        self.request_data(invs, peer)
    }

    /// Send `getdata` message to given `peer`.
    fn request_data(&self, invs: Vec<Inventory>, peer: &mut Connection) -> ProcessResult
    {
        let msg = OutgoingMessage::GetData(invs);
        if peer.send_msg(msg).is_err() {
            ProcessResult::Ban
        } else {
            ProcessResult::Ack
        }
    }

    pub fn recv_block(&mut self, block: Block, peer: &mut Connection) -> ProcessResult
    {
        info!("Process incoming block");
        match self.blockchain.try_add(block) {
            Ok(_) => {
                self.publish_current_blockchain();
                ProcessResult::Ack
            },
            Err(_) => {
                warn!("Peer {:?} send us unwanted block. So we disconnect.", peer);
                ProcessResult::Ban
            },
        }
    }

    /// Publish current blockchain's snapshot to subscribers.
    fn publish_current_blockchain(&mut self)
    {
        fn inner(subscribers: &mut Vec<SyncSender<BlockChain>>, idx: usize, blockchain: BlockChain)
        {
            if subscribers.len() == idx {
                return;
            }

            // Try send blockchain.
            let send_result = {
                let subscriber = subscribers.get_mut(idx).unwrap();
                subscriber.try_send(blockchain.clone())
            };

            // call next process.
            match send_result {
                // If channel is full, simply discard latest update.
                Ok(_) | Err(TrySendError::Full(_)) => inner(subscribers, idx + 1, blockchain),
                Err(TrySendError::Disconnected(_)) => {
                    subscribers.swap_remove(idx);
                    inner(subscribers, idx, blockchain);
                },
            }
        }

        inner(&mut self.subscribers, 0, self.blockchain.freeze())
    }
}

fn check_invs(invs: &[Inventory], blockchain: &BlockChainMut) -> bool
{
    for inv in invs.iter() {
        // Check inventory's type.
        // Should we accept `WitnessBlock` as well?.
        if inv.inv_type != InvType::Block {
            warn!("Incoming inventory's type is not  Block but {:?}", inv.inv_type);
            return false;
        }

        // Check whether given inventory is already stored.
        if blockchain.get_block(&inv.hash).is_some() {
            warn!("Incoming inventory is already stored.");
            return false;
        }
    }

    true
}
