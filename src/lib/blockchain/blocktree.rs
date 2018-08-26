use bitcoin::util::hash::Sha256dHash;
use bitcoin::blockdata::block::Block;
use std::ptr::NonNull;

use super::{BlockData, BlockGenerator, NotFoundPrevBlock, RawBlockData};


/// A honest implementation of blockchain.
pub struct BlockTree<B, G>
{
    // Nodes of current active chain
    active_nodes: Vec<NonNull<Node<B>>>,

    // Generator which is used when a new block is added
    block_generator: G,
}

#[derive(Debug)]
struct Node<B>
{
    prev: Option<NonNull<Node<B>>>,
    nexts: Vec<NonNull<Node<B>>>,
    block: B,
}

impl<B, G> BlockTree<B, G>
where
    B: BlockData,
    G: BlockGenerator<BlockData = B>,
{
    /// # Note
    /// Does not check blockchain validity
    ///
    /// # Panic
    /// if a length of `blocks` is 0.
    pub fn with_initial(blocks: Vec<B>, generator: G) -> BlockTree<B, G>
    {
        assert!(blocks.len() > 0);

        let mut nodes: Vec<NonNull<Node<B>>> = blocks.into_iter().map(Node::new).collect();

        {
            // updaet `prev` field
            let nodes_cloned = nodes.clone();
            let nodes_skip_first = nodes.iter_mut().skip(1);
            for (node, prev) in nodes_skip_first.zip(nodes_cloned) {
                unsafe { node.as_mut().prev = Some(prev) };
            }
        }

        {
            // update `nexts` field
            let nodes_skip_first = nodes.clone().into_iter().skip(1);
            for (node, next) in nodes.iter_mut().zip(nodes_skip_first) {
                unsafe { node.as_mut().nexts.push(next) };
            }
        }

        BlockTree {
            active_nodes: nodes,
            block_generator: generator,
        }
    }

    pub fn try_add(&mut self, block: Block) -> Result<(), NotFoundPrevBlock>
    {
        /* Defines some useful function */

        // Returns last common `Node` between `active_chain` and `node_ptr`'s branch.
        fn find_last_common<B: BlockData>(active_chain: ActiveChain<B>, node_ptr: NonNull<Node<B>>)
            -> NonNull<Node<B>>
        {
            let node = unsafe { node_ptr.as_ref() };
            if active_chain.contains(&node.block) {
                return node_ptr;
            }
            match node.prev {
                None => unreachable!(), // because independent branch never exist.
                Some(prev) => find_last_common(active_chain, prev),
            }
        }

        // Rewinded `active_chain` contains a node whose height is `rewind_height`.
        // Length of `active_chain` must be long enough.
        fn rewind_active_chain<B: BlockData>(active_chain: &mut Vec<NonNull<Node<B>>>, rewind_height: usize)
        {
            unsafe {
                let start_height = active_chain[0].as_ref().block.height();
                let rewind_idx = rewind_height - start_height + 1;
                active_chain.set_len(rewind_idx);
            }
        }

        fn append_nodes<B>(active_chain: &mut Vec<NonNull<Node<B>>>, node_ptr: NonNull<Node<B>>)
        {
            unsafe {
                let node = node_ptr.as_ref();
                let prev_node = node.prev.unwrap();
                if prev_node != *active_chain.last().unwrap() {
                    append_nodes(active_chain, prev_node);
                }
                active_chain.push(node_ptr);
            }
        }

        /* logic starts from here */

        // Search prev block of given block
        let prev_node = match self.find_block(block.header.prev_blockhash) {
            None => return Err(NotFoundPrevBlock(block)),
            Some(node) => node,
        };

        // Generates `BlockData`.
        let prev_block_height = unsafe { prev_node.as_ref().block.height() };
        let new_block_height = prev_block_height + 1;
        let raw_block_data = RawBlockData::new(block, new_block_height);
        let block_data = self.block_generator.generate_block(raw_block_data);

        // Creates a new node
        let new_node = Node::append_block(prev_node, block_data);

        // If new_node is a new tip, replace
        let tail_block_height = unsafe { self.active_nodes.last().unwrap().as_ref().block.height() };
        if tail_block_height < new_block_height {
            // Rewinds current active chain
            let last_common_node = find_last_common(self.active_chain(), new_node);
            let rewind_height = unsafe { last_common_node.as_ref().block.height() };
            rewind_active_chain(&mut self.active_nodes, rewind_height);
            append_nodes(&mut self.active_nodes, new_node);
        }

        Ok(())
    }

    /// Find a block whose bitcoin_hash is equal to given hash
    /// It is depth first search.
    fn find_block(&self, hash: Sha256dHash) -> Option<NonNull<Node<B>>>
    {
        fn inner<B: BlockData>(node_ptr: NonNull<Node<B>>, hash: Sha256dHash) -> Option<NonNull<Node<B>>>
        {
            let node = unsafe { node_ptr.as_ref() };

            for next in node.nexts.iter() {
                if let Some(node) = inner(*next, hash) {
                    return Some(node);
                }
            }

            if node.block.bitcoin_hash() == hash {
                return Some(node_ptr);
            }

            None
        }

        inner(self.active_nodes[0], hash)
    }

    pub fn active_chain(&self) -> ActiveChain<B>
    {
        ActiveChain {
            nodes: &self.active_nodes,
        }
    }

    /// Pop head block.
    /// # Panic
    /// if the number of block contained is 1.
    pub fn pop_head_unchecked(&mut self) -> B
    {
        let poped_head = self.active_nodes.remove(0);
        let mut next_head = self.active_nodes[0]; // panic if length is 1.

        // Drop nodes which will be dangling.
        for may_drop_node in unsafe { poped_head.as_ref().nexts.iter() } {
            if *may_drop_node != next_head {
                unsafe { drop_with_sub_node(*may_drop_node) };
            }
        }

        unsafe {
            next_head.as_mut().prev = None;
        }

        unsafe { Node::into_block(Box::from_raw(poped_head.as_ptr())) }
    }
}

impl<B, G> Drop for BlockTree<B, G>
{
    fn drop(&mut self)
    {
        unsafe { drop_with_sub_node(self.active_nodes[0]) };
    }
}

impl<B> Node<B>
{
    fn new(block: B) -> NonNull<Node<B>>
    {
        let new_node = Node {
            prev: None,
            nexts: vec![],
            block,
        };
        unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(new_node))) }
    }

    fn append_block(mut node: NonNull<Node<B>>, block: B) -> NonNull<Node<B>>
    {
        let new_node = Node {
            prev: Some(node.clone()),
            nexts: vec![],
            block,
        };
        let new_node_ptr = unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(new_node))) };

        // lifetime is valid
        unsafe { node.as_mut().nexts.push(new_node_ptr.clone()) };

        new_node_ptr
    }

    fn into_block(self: Box<Self>) -> B
    {
        self.block
    }
}

unsafe fn drop_with_sub_node<B>(node_ptr: NonNull<Node<B>>)
{
    for next in node_ptr.as_ref().nexts.iter() {
        drop_with_sub_node(*next);
    }
    drop(Box::from_raw(node_ptr.as_ptr()));
}

pub struct ActiveChain<'a, B: 'a>
{
    // TODO : Need non-alocation way
    nodes: &'a Vec<NonNull<Node<B>>>,
}

impl<'a, B: BlockData> ActiveChain<'a, B>
{
    pub fn len(&self) -> usize
    {
        self.nodes.len()
    }

    pub fn get_block(&self, height: usize) -> Option<&B>
    {
        let start_height = self.iter().next().unwrap().height();
        if start_height < height {
            return None;
        }
        self.nodes
            .get(height - start_height)
            .map(|p| unsafe { &p.as_ref().block })
    }

    pub fn contains(&self, b: &B) -> bool
    {
        self.get_block(b.height()).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = &B> + DoubleEndedIterator
    {
        self.nodes.iter().map(|node| unsafe { &node.as_ref().block })
    }
}

/// TODO: Should test re-org case
#[cfg(test)]
mod tests
{
    use super::*;
    use blockchain::HeaderOnlyBlockData;
    use bitcoin::blockdata::block::{Block, BlockHeader};
    use bitcoin::network::serialize::BitcoinHash;

    fn dummy_block_header(prev_hash: Sha256dHash) -> BlockHeader
    {
        let header = BlockHeader {
            version: 1,
            prev_blockhash: prev_hash,
            merkle_root: Sha256dHash::default(),
            time: 0,
            bits: 0,
            nonce: 0,
        };
        header
    }

    #[test]
    fn blocktree_try_add()
    {
        let start_block_header = dummy_block_header(Sha256dHash::default());
        let next_block_header = dummy_block_header(start_block_header.bitcoin_hash());
        let start_block = HeaderOnlyBlockData::new(start_block_header, 0);
        let next_block = Block {
            header: next_block_header,
            txdata: Vec::new(),
        };
        let mut blocktree = BlockTree::with_initial(vec![start_block], |raw: RawBlockData| {
            HeaderOnlyBlockData::new(raw.block.header, raw.height)
        });

        assert_eq!(blocktree.active_chain().len(), 1);

        blocktree.try_add(next_block).unwrap(); // Should success.

        assert_eq!(blocktree.active_chain().len(), 2);

        let headers: Vec<_> = blocktree
            .active_nodes
            .iter()
            .map(|node| unsafe { node.as_ref().block.header().clone() })
            .collect();
        assert_eq!(headers, vec![start_block_header, next_block_header]);
    }
}
