use std::net::{IpAddr, Ipv4Addr};
use bevy::{
    log::{Level, LogPlugin}, 
    prelude::*
};
use bytes::Bytes;
use bevy_dtls::client::{
    cert_option::ClientCertOption, 
    dtls_client::*, 
    health::DtlsClientError, 
    plugin::DtlsClientPlugin
};

#[derive(Resource)]
struct ClientHellooonCounter(u64);

fn send_hellooon_system(
    dtls_client: Res<DtlsClient>, 
    mut counter: ResMut<ClientHellooonCounter>
) {
    let str = format!("from client helloooooon {}", counter.0);
    let msg = Bytes::from(str);
    match dtls_client.send(msg) {
        Ok(_) => counter.0 += 1, 
        Err(e) => error!("{e}")
    }
}

fn recv_hellooon_system(mut dtls_client: ResMut<DtlsClient>) {
    loop {
        let Some(bytes) = dtls_client.recv() else {
            return;
        };

        let msg = String::from_utf8(bytes.to_vec())
        .unwrap();
        info!("message: {msg}");
    }
}

fn handle_net_error(mut errors: EventReader<DtlsClientError>) {
    for e in errors.read() {
        error!("{e:?}");
    }
}

struct ClientPlugin {
    server_addr: IpAddr,
    server_port: u16,
    client_addr: IpAddr,
    client_port: u16,
    cert_option: ClientCertOption
}

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        let mut dtls_client = app.world_mut()
        .resource_mut::<DtlsClient>();
    
        if let Err(e) = dtls_client.start(DtlsClientConfig{ 
            server_addr: self.server_addr, 
            server_port: self.server_port,
            client_addr: self.client_addr, 
            client_port: self.client_port,
            cert_option: self.cert_option.clone()
        }) {
            panic!("{e}")
        }

        app.insert_resource(ClientHellooonCounter(0))
        .add_systems(Update, (
            handle_net_error,
            recv_hellooon_system,
            send_hellooon_system
        ).chain());
    }
}

fn main() {
    App::new()
    .add_plugins((
        DefaultPlugins.set(LogPlugin{
            level: Level::INFO,
            ..default()
        }),
        DtlsClientPlugin{
            buf_size: 512,
            timeout_secs: 10
        }
    ))
    .add_plugins(
        ClientPlugin{
            server_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            server_port: 4443,
            client_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            client_port: 0,
            cert_option: ClientCertOption::Insecure
        }
    )
    .run();
}
