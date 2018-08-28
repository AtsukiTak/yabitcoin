use futures::future::{loop_fn, Future, Loop};

use connection::Connection;
use blockchain::BlockChain;
use error::{Error, ErrorKind};
use super::getheaders;

/// Sync given `BlockChain` with latest blockchain.
/// This process only syncs `BlockHeader`.
/// If you want `Block` as well, please use `process::getblocks` function.
pub fn sync_blockchain(
    conn: Connection,
    block_chain: BlockChain,
) -> impl Future<Item = (Connection, BlockChain), Error = Error>
{
    const MAX_HEADERS_IN_MSG: usize = 2000;

    loop_fn(
        (conn, block_chain), // Initial state
        |(conn, mut block_chain)| {
            let locator_hashes = block_chain.active_chain().locator_hashes_vec();
            getheaders(conn, locator_hashes).and_then(move |(conn, headers)| {
                info!("Received new {} headers", headers.len());

                let is_completed = headers.len() != MAX_HEADERS_IN_MSG;

                for header in headers {
                    if let Err(_) = block_chain.try_add(header) {
                        return Err(Error::from(ErrorKind::MisbehaviorPeer(conn)));
                    }
                }

                match is_completed {
                    true => Ok(Loop::Break((conn, block_chain))),
                    false => Ok(Loop::Continue((conn, block_chain))),
                }
            })
        },
    )
}