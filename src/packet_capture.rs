/*
 * packet-capture.rs - A simple packet capture program
 *                    that captures RTP packets and
 *                    processes them as needed.
*/

#[cfg(not(all(feature = "dpdk_enabled", target_os = "linux")))]
#[allow(dead_code)]
mod packet_capture {
    use std::collections::VecDeque;
    trait Packet {}
    pub struct PacketCapture {
        rtp_bundle: VecDeque<Box<dyn Packet>>,
    }
    impl PacketCapture {
        pub fn new() -> Self {
            PacketCapture {
                rtp_bundle: VecDeque::new(),
            }
        }
        pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
            println!("Packet capture is not supported on this platform");
            Ok(())
        }
    }
}

#[cfg(all(feature = "dpdk_enabled", target_os = "linux"))]
mod packet_capture {
    use capsule::config::{load_config, RuntimeConfig};
    use capsule::packets::{Ethernet, Ip, IpProtocol, Packet, Udp};
    use capsule::runtime::{self, Runtime};
    use std::collections::VecDeque;
    use std::net::Ipv4Addr;

    const RTP_PAYLOAD_TYPE: u8 = 96; // Adjust as needed

    pub struct PacketCapture {
        rtp_bundle: VecDeque<Box<dyn Packet>>,
    }

    impl PacketCapture {
        pub fn new() -> Self {
            PacketCapture {
                rtp_bundle: VecDeque::new(),
            }
        }

        pub fn run(&mut self) -> Result<(), Box<dyn std::error::Error>> {
            let config = load_config()?;
            let mut runtime = Runtime::new(config)?;

            runtime.execute(|port_queue| {
                let mut packets = Vec::new();

                while let Ok(received) = port_queue.dequeue(&mut packets) {
                    for packet in packets.drain(..) {
                        if let Some(rtp_packet) = self.process_packet(packet)? {
                            self.rtp_bundle.push_back(rtp_packet);
                            // Process the bundle as needed
                            // ...
                        }
                    }
                }
                Ok(())
            })?;

            Ok(())
        }

        fn process_packet(
            &self,
            packet: Box<dyn Packet>,
        ) -> Result<Option<Box<dyn Packet>>, Box<dyn std::error::Error>> {
            let ethernet = packet.parse::<Ethernet>()?;
            if let Ok(ipv4) = ethernet.parse::<Ip>() {
                if ipv4.protocol() == IpProtocol::Udp {
                    let udp = ipv4.parse::<Udp>()?;
                    let target_ip: Ipv4Addr = "192.168.1.1".parse()?;
                    let target_port = 1234;

                    if udp.dst_port() == target_port && ipv4.dst() == target_ip {
                        if is_rtp_packet(&udp.payload()) {
                            return Ok(Some(udp.deparse()));
                        }
                    }
                }
            }
            Ok(None)
        }
    }

    fn is_rtp_packet(payload: &[u8]) -> bool {
        !payload.is_empty() && payload[0] == RTP_PAYLOAD_TYPE
    }
}

/*fn main() {
let mut capture = packet_capture::PacketCapture::new();
if let Err(e) = capture.run() {
    eprintln!("Error running packet capture: {}", e);
}
}*/
