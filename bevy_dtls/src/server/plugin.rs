use anyhow::anyhow;
use bevy::prelude::*;
use rustls::crypto::aws_lc_rs;
use super::{
    dtls_server::DtlsServer, 
    event::{self, DtlsServerEvent}
};

fn accept_system(
    mut dtls_server: ResMut<DtlsServer>,
    mut errors: EventWriter<DtlsServerEvent>
) {
    if dtls_server.is_closed() {
        return;
    }
    
    loop {
        let Some(conn_idx) = dtls_server.acpt() else {
            return;
        };
    
        if let Err(e) = dtls_server.start_conn(conn_idx) {
            errors.send(DtlsServerEvent::Error { 
                err: anyhow!("conn {conn_idx:?} could not be started: {e}") 
            });

            continue;
        }
    
        debug!("conn {conn_idx:?} has been started from default system");
    }
}

pub struct DtlsServerPlugin {
    pub max_clients: usize,
    pub buf_size: usize,
    pub send_timeout_secs: u64,
    pub recv_timeout_secs: Option<u64>
}

impl Plugin for DtlsServerPlugin {
    fn build(&self, app: &mut App) {
        if aws_lc_rs::default_provider()
        .install_default()
        .is_err() {
            panic!("failed to setup crypto provider");
        }

        let dtls_server = match DtlsServer::new(
            self.max_clients,
            self.buf_size, 
            self.send_timeout_secs,
            self.recv_timeout_secs
        ) {
            Ok(s) => s,
            Err(e) => panic!("{e}")
        };

        app.insert_resource(dtls_server)
        .add_event::<DtlsServerEvent>()
        .add_systems(PreUpdate, accept_system)
        .add_systems(PostUpdate, (
            event::health_event_system,
            event::timeout_event_system
        ).chain());
    }
}
