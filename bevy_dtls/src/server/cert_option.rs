use rustls::RootCertStore;
use webrtc_dtls::{
    config::{ClientAuthType, Config, ExtendedMasterSecretType}, 
    crypto::Certificate
};
use crate::cert::loader;

#[derive(Clone)]
pub enum ServerCertOption {
    GenerateSelfSigned {
        subject_alt_name: &'static str
    },
    Load {
        priv_key_path: &'static str,
        certificate_path: &'static str,
        client_ca_path: &'static str
    }
}

impl ServerCertOption {
    pub fn to_dtls_config(self) -> anyhow::Result<Config> {
        let config = match self {
            ServerCertOption::GenerateSelfSigned { 
                subject_alt_name 
            } => {
                let cert = Certificate::generate_self_signed(
                    vec![subject_alt_name.to_string()]    
                )?;

                Config{
                    certificates: vec![cert],
                    extended_master_secret: ExtendedMasterSecretType::Require,
                    ..Default::default()
                }
            }
            ServerCertOption::Load { 
                priv_key_path, 
                certificate_path,
                client_ca_path 
            } => {
                let cert = loader::load_key_and_certificate(
                    priv_key_path.into(),
                    certificate_path.into()
                )?;
                
                let mut client_ca_store = RootCertStore::empty();
                let client_ca = loader::load_certtificate(client_ca_path.into())?;
                for c in client_ca.iter() {
                    client_ca_store.add(c.clone())?;
                }   

                Config{
                    certificates: vec![cert],
                    extended_master_secret: ExtendedMasterSecretType::Require,
                    client_auth: ClientAuthType::RequireAndVerifyClientCert,
                    client_cas: client_ca_store,
                    ..Default::default()
                }
            }
        };

        Ok(config)
    }
}
