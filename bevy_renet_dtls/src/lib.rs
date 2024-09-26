pub mod server;
pub mod client;

pub use bevy_dtls as dtls;
use bevy::prelude::SystemSet;
use bevy_renet::renet::ClientId;
use bevy_dtls::server::dtls_server::ConnIndex;

pub trait ConnIndexRenetExt {
    fn to_renet_id(&self) -> ClientId;
    fn from_renet_id(id: &ClientId) -> Self; 
}

impl ConnIndexRenetExt for ConnIndex {
    fn to_renet_id(&self) -> ClientId {
        ClientId::from_raw(self.index())
    }

    fn from_renet_id(id: &ClientId) -> Self {
        ConnIndex::new(id.raw())
    }
} 

#[derive(SystemSet, Eq, PartialEq, Debug, Clone, Hash)]
pub enum DtlsSet {
    Acpt,
    Recv,
    Send
}
