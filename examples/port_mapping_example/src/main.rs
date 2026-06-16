use bittorrent_rs::network::nat::{FallbackPortMapper, PortMapper, get_default_gateway};
use std::time::Duration;

fn main() {
    println!("=== NAT Port Mapping Example ===");

    // 1. Determine the gateway IP address automatically
    let gateway = get_default_gateway();
    println!("Inferred local router gateway IP: {}", gateway);

    // 2. Instantiate the fallback port mapper (tries NAT-PMP, then UPnP/SOAP)
    let mapper = FallbackPortMapper::new(gateway);

    let internal_port = 6881;
    let external_port = 6881;
    let lifetime_secs = 3600; // 1 hour lease time

    // 3. Request TCP Port Mapping
    println!("\nRequesting TCP port mapping: {} -> {}", external_port, internal_port);
    match mapper.request_mapping(true, internal_port, external_port, lifetime_secs) {
        Ok(mapped_port) => {
            println!("Successfully mapped TCP port! External accessible port: {}", mapped_port);
            
            // 4. Request UDP Port Mapping
            println!("Requesting UDP port mapping: {} -> {}", external_port, internal_port);
            match mapper.request_mapping(false, internal_port, external_port, lifetime_secs) {
                Ok(udp_mapped_port) => {
                    println!("Successfully mapped UDP port! External accessible port: {}", udp_mapped_port);
                }
                Err(e) => {
                    println!("UDP port mapping failed: {:?}", e);
                }
            }

            // Simulate holding the port for a short duration
            println!("\nKeeping ports open for 5 seconds...");
            std::thread::sleep(Duration::from_secs(5));

            // 5. Clean up by releasing mapping
            println!("Releasing port mapping for TCP/UDP port {}", internal_port);
            if let Err(e) = mapper.release_mapping(true, internal_port) {
                println!("Failed to release TCP mapping: {:?}", e);
            } else {
                println!("TCP port mapping released.");
            }

            if let Err(e) = mapper.release_mapping(false, internal_port) {
                println!("Failed to release UDP mapping: {:?}", e);
            } else {
                println!("UDP port mapping released.");
            }
        }
        Err(e) => {
            println!("TCP Port mapping request failed (no NAT-PMP or UPnP router found/active): {:?}", e);
        }
    }
}
