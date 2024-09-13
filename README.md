# bevy_renet_dtls
dtls encryption transport for bevy_renet game networking as alternative of netcode.  

from technical view, this is ECS port of webrtc_dtls and taking advantage of it for game networking with reliable UDP.  

#### replicon simple box demo  
popular(!?) demo with bevy_replicon & bevy_replicon_renet  
server:`cargo run --package replicon_demo -- server`  
client: `cargo run --package replicon_demo -- client`  

#### demo with bevy_renet
simple messaging demo without higher level replication systems  
server:`cargo run --bin reliable_server`  
client: `cargo run --bin reliable_client`  

#### demo without renet
simple messaging demo without reliable UDP systems  
server:`cargo run --bin unreliable_server`  
client: `cargo run --bin unreliable_client`

