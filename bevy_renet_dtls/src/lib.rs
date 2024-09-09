pub mod server {
    pub mod renet_dtls_server;
}
pub mod client {
    pub mod renet_dtls_client;
}

use bevy_renet::renet::ClientId;
use bevy_dtls::server::dtls_server::ConnIndex;

pub trait ToRenetClientId {
    fn renet_client_id(&self) -> ClientId;
}

impl ToRenetClientId for ConnIndex {
    fn renet_client_id(&self) -> ClientId {
        ClientId::from_raw(self.index() as u64)
    }
}
