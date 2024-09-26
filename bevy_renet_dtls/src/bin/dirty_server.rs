use std::{
    net::{IpAddr, Ipv4Addr}, 
    time::Duration
};
use bevy::{
    app::ScheduleRunnerPlugin, 
    log::{Level, LogPlugin}, 
    prelude::*, utils::HashMap
};
use bevy_renet::{
    renet::{ClientId, ConnectionConfig, DefaultChannel, RenetServer}, 
    RenetServerPlugin
};
use bevy_dtls::server::{
    cert_option::ServerCertOption, 
    dtls_server::{DtlsServer, DtlsServerConfig}, 
    health::{DtlsServerClosed, DtlsServerError}
};
use bevy_renet_dtls::{server::RenetDtlsServerPlugin, ConnIndexRenetExt};
use bytes::Bytes;

#[derive(Resource)]
struct SentHellooonCounter(u64);

#[derive(Resource)]
struct ReceivedHellooonCounter(HashMap<ClientId, u64>);

fn send_hellooon_system(
    mut renet_server: ResMut<RenetServer>,
    dtls_server: Res<DtlsServer>,
    mut counter: ResMut<SentHellooonCounter>
) {
    let renet_len = renet_server.connected_clients();
    let dtls_len = dtls_server.connected_clients();
    
    if renet_len != dtls_len {
        warn!("connected clients mismatch, renet: {renet_len}, dtls: {dtls_len}");
    }
    if renet_len == 0 {
        return;
    }

    let str = format!("from server helloooooon {}", counter.0);
    let msg = Bytes::from(str);
    renet_server.broadcast_message(DefaultChannel::ReliableOrdered, msg);
    counter.0 += 1;
    info!("broadcasted: {}", counter.0);

    // if counter.0 > 10 {
    //     warn!("disconnecting all");
    //     renet_server.disconnect_all();
    //     dtls_server.stop();
    //     counter.0 = 0;
    // }
}

fn recv_hellooon_system(
    mut renet_server: ResMut<RenetServer>,
    mut counter: ResMut<ReceivedHellooonCounter>
) {
    let ch_len = 3_u8;
    let clients = renet_server.clients_id();

    for client_id in clients {
        for ch in 0..ch_len {
            loop {
                let Some(bytes) = renet_server.receive_message(client_id, ch) else {
                    break;
                };
    
                let msg = String::from_utf8(bytes.to_vec()).unwrap();
                
                let count = counter.0.entry(client_id)
                .or_default();
                info!("message from: {client_id}: {msg}: {count}");
                
                *count += 1;
                // if *count > 100  {
                //     warn!("disconnecting: {client_id:?}");
                //     renet_server.disconnect(client_id);
                // }
            }
        }
    }    
}

fn handle_net_error(
    mut errors: EventReader<DtlsServerError>,
    mut renet_server: ResMut<RenetServer>
) {
    for e in errors.read() {
        match e {
            DtlsServerError::SendTimeout { conn_index, .. } => {
                warn!("send timeout: disconnecting");
                renet_server.disconnect(conn_index.to_renet_id());
            }
            DtlsServerError::RecvTimeout { conn_index } => {
                warn!("recv timeout: disconnecting");
                renet_server.disconnect(conn_index.to_renet_id());
            }
            DtlsServerError::Error { err } => {
                // i found duplicate binding error event after
                // closing listener. i will try again later but
                // all i can do for now is just panic
                error!("{err}");
            }
            DtlsServerError::ConnError { conn_index, err } => {
                // better way to get this specific error ??
                if err.to_string()
                .ends_with("Alert is Fatal or Close Notify") {
                    warn!("client {conn_index:?} disconnected: {err}");
                } else {
                    error!("{err}: disconnecting");
                }

                renet_server.disconnect(conn_index.to_renet_id());
            }
        }
    }
}

fn handle_closed(
    mut dtls_server: ResMut<DtlsServer>,
    mut closed: EventReader<DtlsServerClosed>
) {
    for _ in closed.read() {
        if let Err(e) = dtls_server.restart() {
            panic!("{e}");
        }

        info!("server is restarted");
    }
}

struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        let mut dtls_server = app.world_mut()
        .resource_mut::<DtlsServer>();
        if let Err(e) = dtls_server.start(DtlsServerConfig{
            listen_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            listen_port: 4443,
            cert_option: ServerCertOption::Load { 
                priv_key_path: "my_certificates/server.priv.pem", 
                certificate_path: "my_certificates/server.pub.pem",
            }
        }) {
            panic!("{e}");
        }

        let renet_server = RenetServer::new(ConnectionConfig::default());
        app.insert_resource(renet_server)
        .insert_resource(SentHellooonCounter(0))
        .insert_resource(ReceivedHellooonCounter(default()))
        .add_systems(Update, (
            handle_net_error,
            send_hellooon_system
            .run_if(resource_exists::<RenetServer>),
            recv_hellooon_system
            .run_if(resource_exists::<RenetServer>),
            handle_closed
            .run_if(resource_exists::<RenetServer>),
        ).chain());

        info!("server is started");
    }
}

fn main() {
    App::new()
    .add_plugins((
        MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(
            Duration::from_secs_f32(1.0 / 30.0)
        )),
        LogPlugin{
            level: Level::INFO,
            ..default()
        },
        RenetServerPlugin,
        RenetDtlsServerPlugin{
            max_clients: 1,
            buf_size: 1500,
            send_timeout_secs: 1,
            recv_timeout_secs: Some(1)
        }
    ))
    .add_plugins(ServerPlugin)
    .run();
}
