use anyhow::anyhow;
use bevy::prelude::*;
use bevy_renet::{renet::{ClientId, RenetServer}, RenetReceive, RenetSend};
use bevy_dtls::server::{
    dtls_server::{ConnIndex, DtlsServer}, 
    event::{self, DtlsServerEvent}
};
use bytes::Bytes;
use rustls::crypto::aws_lc_rs;
use crate::{ConnIndexRenetExt, DtlsSet};

pub trait RenetServerDtlsExt {
    fn disconnect_dtls(
        &mut self, 
        dtls_server: &mut DtlsServer, 
        conn_index: u64
    );

    fn disconnect_all_dtls(&mut self, dtls_server: &mut DtlsServer);
}

impl RenetServerDtlsExt for RenetServer {
    #[inline]
    fn disconnect_dtls(
        &mut self, 
        dtls_server: &mut DtlsServer, 
        conn_index: u64
    ) {
        let client_id = ClientId::from_raw(conn_index);
        self.disconnect(client_id);
        dtls_server.disconnect(conn_index);
        self.remove_connection(client_id);
    }

    fn disconnect_all_dtls(&mut self, dtls_server: &mut DtlsServer) {
        let indices = dtls_server.client_indices();
        for idx in indices {
            self.disconnect_dtls(dtls_server, idx);         
        }
    }
}

fn acpt_system(
    mut renet_server: ResMut<RenetServer>,
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

        debug!("conn: {conn_idx:?} has been started from renet-dtls system");

        renet_server.add_connection(conn_idx.to_renet_id());
    }
}

fn recv_system(
    mut renet_server: ResMut<RenetServer>,
    mut dtls_server: ResMut<DtlsServer>,
    mut errors: EventWriter<DtlsServerEvent>
) {
    if dtls_server.is_closed() {
        return;
    }

    loop {
        let Some((conn_idx, bytes)) = dtls_server.recv() else {
            return;
        };

        if let Err(e) = renet_server.process_packet_from(
            &bytes, 
            conn_idx.to_renet_id()
        ) {
            errors.send(DtlsServerEvent::ConnError { 
                conn_index: conn_idx, 
                err: anyhow!("error on receiving conn {conn_idx:?}: {e}")
            });
        }
    }
}

fn send_system(
    mut renet_server: ResMut<RenetServer>,
    dtls_server: Res<DtlsServer>,
    mut errors: EventWriter<DtlsServerEvent>
) {
    if dtls_server.is_closed() {
        return;
    }

    let clients = renet_server.clients_id();
    'client_loop: for client_id in clients {
        // no packets will be sent if renet server is closed before this system, 
        // even though send_message is called on this frame
        let packets = renet_server.get_packets_to_send(client_id)
        .unwrap();

        for pkt in packets {
            if let Err(e) = dtls_server.send(client_id.raw(), Bytes::from(pkt)) {
                errors.send(DtlsServerEvent::ConnError { 
                    conn_index: ConnIndex::from_renet_id(&client_id), 
                    err: anyhow!("error on sending to conn {client_id}: {e}") 
                });

                continue 'client_loop;
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
        .add_event::<DtlsServerEvent>()
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
            event::health_event_system,
            event::timeout_event_system
        )
            .chain()
            .after(DtlsSet::Send)
            .run_if(resource_exists::<RenetServer>)
        );
    }
}
