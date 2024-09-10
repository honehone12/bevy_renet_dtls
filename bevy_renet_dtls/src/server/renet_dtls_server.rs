use bevy::prelude::*;
use bevy_renet::{renet::RenetServer, RenetServerPlugin};
use bevy_dtls::server::dtls_server::DtlsServer;
use bytes::Bytes;
use rustls::crypto::aws_lc_rs;
use crate::ToRenetClientId;

fn acpt_system(
    mut renet_server: ResMut<RenetServer>,
    mut dtls_server: ResMut<DtlsServer>
) {
    loop {
        let Some(conn_idx) = dtls_server.acpt() else {
            break;
        };

        if let Err(e) = dtls_server.start_conn(conn_idx) {
            if cfg!(debug_assertions) {
                panic!("{e}")
            } else {
                error!("{e}");
                continue;
            }
        }

        debug!(
            "conn: {} has been started from renet-dtls system", 
            conn_idx.index()
        );

        renet_server.add_connection(conn_idx.renet_client_id());
    }
}

fn recv_system(
    mut renet_server: ResMut<RenetServer>,
    mut dtls_server: ResMut<DtlsServer>
) {
    loop {
        let Some((conn_idx, bytes)) = dtls_server.recv() else {
            return;
        };

        if let Err(e) = renet_server.process_packet_from(
            &bytes, 
            conn_idx.renet_client_id()
        ) {
            if cfg!(debug_assertions) {
                panic!("{e}")
            } else {
                error!("{e}");
            }
        }
    }
}

fn send_system(
    mut renet_server: ResMut<RenetServer>,
    dtls_server: Res<DtlsServer>
) {
    let clients = renet_server.clients_id();
    'client_loop: for client_id in clients {
        let packets = renet_server.get_packets_to_send(client_id)
        .unwrap();

        for pkt in packets {
            if let Err(e) = dtls_server.send(client_id.raw(), Bytes::from(pkt)) {
                if cfg!(debug_assertions) {
                    panic!("{e}")
                } else {
                    error!("{e}");
                    continue 'client_loop;
                }
            }
        }
    }
}

pub struct RenetDtlsServerPlugin {
    pub buf_size: usize,
    pub send_timeout_secs: u64,
    pub recv_timeout_secs: Option<u64>
}

impl Plugin for RenetDtlsServerPlugin {
    fn build(&self, app: &mut App) {
        if aws_lc_rs::default_provider()
        .install_default()
        .is_err() {
            panic!("failed to setup crypto provider");
        }

        let dtls_server = match DtlsServer::new(
            self.buf_size,
            self.send_timeout_secs,
            self.recv_timeout_secs
        ) {
            Ok(s) => s,
            Err(e) => panic!("{e}")
        };

        app.add_plugins(RenetServerPlugin)
        .insert_resource(dtls_server)
        .add_systems(PreUpdate, (
            acpt_system,
            recv_system,
        ))
        .add_systems(PostUpdate, send_system);
    }
}
