use bevy::prelude::*;
use bevy_renet::{renet::RenetClient, RenetReceive, RenetSend};
use bevy_dtls::client::{
    dtls_client::{DtlsClient, DtlsClientConfig}, 
    health::{self, DtlsClientError}
};
use bytes::Bytes;
use rustls::crypto::aws_lc_rs;
use crate::DtlsSet;

pub trait RenetClientDtlsExt {
    fn start_with_dtls(
        &mut self,
        dtls_client: &mut DtlsClient,
        config: DtlsClientConfig
    ) -> anyhow::Result<()>;

    fn disconnect_with_dtls(
        &mut self,
        dtls_client: &mut DtlsClient
    );
}

impl RenetClientDtlsExt for RenetClient {
    #[inline]
    fn start_with_dtls(
        &mut self,
        dtls_client: &mut DtlsClient, 
        config: DtlsClientConfig
    ) -> anyhow::Result<()> {
        self.set_connecting();
        dtls_client.start(config)?;
        self.set_connected();
        Ok(())
    }

    #[inline]
    fn disconnect_with_dtls(
        &mut self,
        dtls_client: &mut DtlsClient
    ) {
        self.disconnect();
        dtls_client.close();    
    }
}

fn send_system(
    mut renet_client: ResMut<RenetClient>,
    dtls_client: Res<DtlsClient>
) {
    // no packets will be sent if renet server is closed before this system, 
    // even though send_message is called on this frame    
    let packets = renet_client.get_packets_to_send();
    for pkt in packets {
        if let Err(e) = dtls_client.send(Bytes::from(pkt)) {
            if cfg!(debug_assertions) {
                panic!("{e}");
            } else {
                error!("{e}");
                return;
            }
        }
    }
}

fn recv_system(
    mut renet_client: ResMut<RenetClient>,
    mut dtls_client: ResMut<DtlsClient>
) {
    loop {
        let Some(bytes) = dtls_client.recv() else {
            return;
        };

        renet_client.process_packet(&bytes);
    }
}

pub struct RenetDtlsClientPlugin {
    pub timeout_secs: u64,
    pub buf_size: usize
}

impl Plugin for RenetDtlsClientPlugin {
    fn build(&self, app: &mut App) {
        if aws_lc_rs::default_provider()
        .install_default()
        .is_err() {
            info!("crypto provider already exists");
        }

        let dtls_client = match DtlsClient::new(self.buf_size, self.timeout_secs) {
            Ok(c) => c,
            Err(e) => panic!("{e}")
        };

        app.insert_resource(dtls_client)
        .add_event::<DtlsClientError>()
        .configure_sets(PreUpdate, DtlsSet::Recv.before(RenetReceive))
        .configure_sets(PostUpdate, DtlsSet::Send.after(RenetSend))
        .add_systems(PreUpdate, 
            recv_system
            .in_set(DtlsSet::Recv)
            .run_if(resource_exists::<RenetClient>)
        )
        .add_systems(PostUpdate, 
            send_system
            .in_set(DtlsSet::Send)
            .run_if(resource_exists::<RenetClient>)
        )
        .add_systems(PostUpdate, (
            health::fatal_event_system,
            health::timeout_event_system
        ).chain(
        ).after(DtlsSet::Send));
    }
}
