use std::{net::{IpAddr, Ipv4Addr}, time::Duration};
use bevy::{
    app::ScheduleRunnerPlugin, 
    log::{Level, LogPlugin}, 
    prelude::*
};
use bevy_dtls::server::{
    cert_option::ServerCertOption, 
    dtls_server::{DtlsServer, DtlsServerConfig, DtlsServerTimeout}, 
    plugin::DtlsServerPlugin
};
use bytes::Bytes;

#[derive(Resource)]
struct ServerHellooonCounter(usize);

fn send_hellooon_system(
    dtls_server: Res<DtlsServer>, 
    mut counter: ResMut<ServerHellooonCounter>
) {
    if dtls_server.clients_len() == 0 {
        return;
    }

    let str = format!("from server helloooooon {}", counter.0);
    let msg = Bytes::from(str);
    match dtls_server.broadcast(msg) {
        Ok(_) => counter.0 += 1, 
        Err(e) => panic!("{e}")
    }
}

fn recv_hellooon_system(mut dtls_server: ResMut<DtlsServer>) {
    loop {
        let Some((idx, bytes)) = dtls_server.recv() else {
            return;
        };

        let msg = String::from_utf8(bytes.to_vec()).unwrap();
        info!("message from conn: {}: {msg}", idx.index());
    }
}

fn timeout_check_system(mut dtls_server: ResMut<DtlsServer>) {
    loop {
        let Err(e) = dtls_server.timeout_check() else {
            return;
        };
    
        match e {
            DtlsServerTimeout::Send { conn_index, bytes: _ } => {
                error!("sender: {conn_index:?} timeout, but still available to re-try");
            }
            DtlsServerTimeout::Recv(idx) => {
                error!("recver: {idx:?} timeout, this is useful for impl heartbeat");
            }
        }
    }
}

fn health_check_system(mut dtls_server: ResMut<DtlsServer>) {
    let health = dtls_server.health_check();
    if let Some(Err(e)) = health.listener {
        panic!("listener: {e}");
    }
    if let Some((idx, Err(e))) = health.sender.get(0) {
        panic!("sender: {idx}: {e}");
    }
    if let Some((idx, Err(e))) = health.recver.get(0) {
        panic!("recver: {idx}: {e}");
    }
}

struct SereverPlugin {
    listen_addr: IpAddr,
    listen_port: u16,
    cert_option: ServerCertOption
}

impl Plugin for SereverPlugin {
    fn build(&self, app: &mut App) {
        let mut dtls_server = app.world_mut()
        .resource_mut::<DtlsServer>();

        if let Err(e) = dtls_server.start(DtlsServerConfig{
            listen_addr: self.listen_addr,
            listen_port: self.listen_port,
            cert_option: self.cert_option
        }) {
            panic!("{e}");
        }
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
        DtlsServerPlugin{
            buf_size: 512,
            send_timeout_secs: 10,
            recv_timeout_secs: Some(10)
        }
    ))
    .add_plugins(SereverPlugin{
        listen_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
        listen_port: 4443,
        cert_option: ServerCertOption::GenerateSelfSigned { 
            subject_alt_name: "webrtc.rs"
        }
        // cert_option: ServerCertOption::Load { 
        //     priv_key_path: "my_certificates/server.priv.pem", 
        //     certificate_path: "my_certificates/server.pub.pem",
        //     client_ca_path: "my_certificates/server.pub.pem" 
        // }
    })
    .insert_resource(ServerHellooonCounter(0))
    .add_systems(Update, (
        send_hellooon_system,
        recv_hellooon_system,
        timeout_check_system,
        health_check_system
    ))
    .run();
}
