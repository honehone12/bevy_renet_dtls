use bevy::prelude::*;
use rustls::crypto::aws_lc_rs;
use super::{
    dtls_client::DtlsClient, 
    health::{self, DtlsClientError}
};

pub struct DtlsClientPlugin {
    pub timeout_secs: u64,
    pub buf_size: usize
}

impl Plugin for DtlsClientPlugin {
    fn build(&self, app: &mut App) {
        if aws_lc_rs::default_provider()
        .install_default()
        .is_err() {
            panic!("failed to set up crypto provider");
        }

        let dtls_client = match DtlsClient::new(self.buf_size, self.timeout_secs) {
            Ok(c) => c,
            Err(e) => panic!("{e}")
        };

        app.insert_resource(dtls_client)
        .add_event::<DtlsClientError>()
        .add_systems(PostUpdate, (
            health::fatal_event_system,
            health::timeout_event_system
        ).chain());
    }
}
