#[cfg(target_os = "linux")]
mod linux {
    use anyhow::{anyhow, Context};
    use std::{error::Error, process::Command};

    const CHAIN_NAME: &str = "cpxy";
    const TPROXY_MARK: u32 = 121;
    const ROUTING_TABLE_NUMBER: u32 = 99;

    fn execute_command(program: &str, args: &[&str]) -> anyhow::Result<String> {
        use std::fmt::Write;

        let mut final_command = program.to_string();

        let mut cmd = Command::new(program);
        for arg in args {
            cmd.arg(arg);
            write!(&mut final_command, " {arg}")?;
        }

        let output = cmd
            .output()
            .with_context(|| format!("Running {final_command}"))?;
        if output.status.success() {
            Ok(String::from_utf8(output.stdout)?)
        } else {
            Err(anyhow!(
                "Error running {final_command}: \n{}",
                String::from_utf8(output.stderr)?
            ))
        }
    }

    pub fn clean_up() -> anyhow::Result<()> {
        let ipt = iptables::new(false).context("Create iptable instance")?;
        let _ = ipt.delete("nat", "PREROUTING", &format!("-j {CHAIN_NAME}"));
        let _ = ipt.flush_chain("nat", CHAIN_NAME);
        let _ = ipt.delete_chain("nat", CHAIN_NAME);

        let _ = ipt.delete("mangle", "PREROUTING", &format!("-j {CHAIN_NAME}"));
        let _ = ipt.flush_chain("mangle", CHAIN_NAME);
        let _ = ipt.delete_chain("mangle", CHAIN_NAME);

        let _ = execute_command(
            "ip",
            &[
                "rule",
                "delete",
                "fwmark",
                &TPROXY_MARK.to_string(),
                "lookup",
                &ROUTING_TABLE_NUMBER.to_string(),
            ],
        );
        let _ = execute_command(
            "ip",
            &["route", "flush", "table", &ROUTING_TABLE_NUMBER.to_string()],
        );

        Ok(())
    }

    pub fn add_rules(tcp_port: u16, udp_port: Option<u16>) -> anyhow::Result<()> {
        execute_command("sysctl", &["-w", "net.ipv4.conf.all.route_localnet=1"])?;
        let ipt = iptables::new(false).context("Create iptable instance")?;

        let networks = [
            "10.0.0.0/8",
            "100.64.0.0/10",
            "127.0.0.0/8",
            "169.254.0.0/16",
            "172.16.0.0/12",
            "192.0.0.0/24",
            "192.168.0.0/16",
            "198.18.0.0/15",
            "255.255.255.255/32",
        ];

        // TCP rules
        ipt.new_chain("nat", CHAIN_NAME)
            .context("Creating new NAT chain")?;
        for network in &networks {
            ipt.append_unique("nat", CHAIN_NAME, &format!("-d {network} -j RETURN"))
                .context(&format!("Creating RETURN rule for local network {network}"))?;
        }
        ipt.append_unique(
            "nat",
            CHAIN_NAME,
            &format!("-p tcp -j DNAT --to-destination 127.0.0.1:{tcp_port}"),
        )
        .context("Creating DNAT rule for TCP")?;
        ipt.append_unique("nat", "PREROUTING", &format!("-j {CHAIN_NAME}"))
            .context("Adding proxy rule to nat/PREROUTING")?;

        // UDP rules
        if let Some(udp_port) = udp_port {
            ipt.new_chain("mangle", CHAIN_NAME)
                .context("Creating new mangle chain")?;
            for network in &networks {
                ipt.append_unique("mangle", CHAIN_NAME, &format!("-d {network} -j RETURN"))
                    .context(&format!("Creating RETURN rule for local network {network}"))?;
            }
            ipt.append_unique(
                "mangle",
                CHAIN_NAME,
                &format!("-p udp -j TPROXY --tproxy-mark {TPROXY_MARK} --on-ip 127.0.0.1 --on-port {udp_port}"),
            )
            .context("Creating TPROXY rule for UDP")?;
            ipt.append_unique("mangle", "PREROUTING", &format!("-j {CHAIN_NAME}"))
                .context("Creating proxy rule for UDP")?;

            // IP rules and routing table for UDP TPROXY
            execute_command(
                "ip",
                &[
                    "rule",
                    "add",
                    "fwmark",
                    &TPROXY_MARK.to_string(),
                    "lookup",
                    &ROUTING_TABLE_NUMBER.to_string(),
                ],
            )
            .context("Adding IP rule for UDP")?;
            execute_command(
                "ip",
                &[
                    "route",
                    "add",
                    "local",
                    "0.0.0.0/0",
                    "dev",
                    "lo",
                    "table",
                    &ROUTING_TABLE_NUMBER.to_string(),
                ],
            )
            .context("Adding IP route for UDP")?;
        }

        Ok(())
    }

    trait StdErrorResult {
        type Item;
        fn context(self, s: &str) -> anyhow::Result<Self::Item>;
    }

    impl<T> StdErrorResult for Result<T, Box<dyn Error>> {
        type Item = T;
        fn context(self, s: &str) -> anyhow::Result<Self::Item> {
            self.map_err(|e| anyhow!("{s}: {e:?}"))
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod noop {
    pub fn add_rules(_tcp_port: u16, _udp_port: Option<u16>) -> anyhow::Result<()> {
        Ok(())
    }

    pub fn clean_up() -> anyhow::Result<()> {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(target_os = "linux"))]
pub use noop::*;
