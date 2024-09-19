use bevy::prelude::*;
use bevy_renet::{renet::RenetServer, RenetReceive, RenetSend};
use bevy_dtls::server::{
    dtls_server::DtlsServer, 
    health::{self, DtlsServerError, DtlsServerClosed}
};
use bytes::Bytes;
use rustls::crypto::aws_lc_rs;
use crate::{DtlsSet, ToRenetClientId};

fn clean_system(
    mut renet_server: ResMut<RenetServer>,
    mut dtls_server: ResMut<DtlsServer>
) {
    let dis_conns = renet_server.disconnections_id();
    for dis_conn in dis_conns {
        dtls_server.disconnect(dis_conn.raw());
        debug!("cleaning: {dis_conn:?}, removed from dtls server");
        renet_server.remove_connection(dis_conn);
        debug!("cleaning: {dis_conn:?}, removed from renet server");
    }
}

fn acpt_system(
    mut renet_server: ResMut<RenetServer>,
    mut dtls_server: ResMut<DtlsServer>
) {
    loop {
        let Some(conn_idx) = dtls_server.acpt() else {
            return;
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
        // no packets will be sent if renet server is closed before this system, 
        // even though send_message is called on this frame
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
    pub max_clients: usize,
    pub buf_size: usize,
    pub send_timeout_secs: u64,
    pub recv_timeout_secs: Option<u64>
}

impl Plugin for RenetDtlsServerPlugin {
    fn build(&self, app: &mut App) {
        if aws_lc_rs::default_provider()
        .install_default()
        .is_err() {
            info!("crypto provider already exists");
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
        .add_event::<DtlsServerError>()
        .add_event::<DtlsServerClosed>()
        .configure_sets(PreUpdate, DtlsSet::Recv.before(RenetReceive))
        .configure_sets(PreUpdate, DtlsSet::Acpt.before(DtlsSet::Recv))
        .configure_sets(PostUpdate, DtlsSet::Send.after(RenetSend))
        .add_systems(PreUpdate, 
            acpt_system
            .in_set(DtlsSet::Acpt)
            .run_if(resource_exists::<RenetServer>)
        )
        .add_systems(PreUpdate, 
            recv_system
            .in_set(DtlsSet::Recv)
            .run_if(resource_exists::<RenetServer>)
        )
        .add_systems(PostUpdate, 
            send_system
            .in_set(DtlsSet::Send)
            .run_if(resource_exists::<RenetServer>)
        )
        .add_systems(PostUpdate, (
            clean_system,
            health::fatal_event_system,
            health::timeout_event_system
        ).chain(
        ).after(DtlsSet::Send
        ).run_if(resource_exists::<RenetServer>));
    }
}
