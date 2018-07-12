use std::time::{SystemTime, UNIX_EPOCH};
use bitcoin::network::{constants, message::NetworkMessage, message_network::VersionMessage};

use socket::SyncSocket;
use error::{Error, ErrorKind};

pub struct Connection {
    socket: SyncSocket,

    remote_version_msg: VersionMessage,
    local_version_msg: VersionMessage,
}

impl Connection {
    pub fn initialize(mut socket: SyncSocket, start_height: i32) -> Result<Connection, Error> {
        // Send Version msg
        let local_version_msg = version_msg(&socket, start_height);
        socket.send_msg(NetworkMessage::Version(local_version_msg.clone()))?;

        // Receive Version msg
        let remote_version_msg = match socket.recv_msg()? {
            NetworkMessage::Version(v) => v,
            msg => {
                error!("Expect Version msg but found {:?}", msg);
                return Err(Error::from(ErrorKind::InvalidPeer));
            }
        };

        // Send Verack msg
        socket.send_msg(NetworkMessage::Verack)?;

        // Receive Verack msg
        match socket.recv_msg()? {
            NetworkMessage::Verack => {}
            msg => {
                error!("Expect Verack msg but found {:?}", msg);
                return Err(Error::from(ErrorKind::InvalidPeer));
            }
        }

        Ok(Connection {
            socket: socket,
            remote_version_msg: remote_version_msg,
            local_version_msg: local_version_msg,
        })
    }
}

fn version_msg(socket: &SyncSocket, start_height: i32) -> VersionMessage {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    VersionMessage {
        version: constants::PROTOCOL_VERSION,
        services: constants::SERVICES,
        timestamp: timestamp,
        receiver: socket.remote_addr().clone(),
        sender: socket.local_addr().clone(),
        nonce: 0,
        user_agent: socket.user_agent().into(),
        start_height: start_height,
        relay: false,
    }
}