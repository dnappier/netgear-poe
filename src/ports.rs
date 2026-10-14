use crate::identity::{self, Attachment, Vendors};
use crate::{client::Switch, config::Config};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize)]
pub struct Port {
    pub unit: u64,
    pub slot: u64,
    pub port: u64,
    pub link_up: Option<bool>,
    pub speed: String,
    pub poe_capable: bool,
    pub poe_enabled: Option<bool>,
    pub poe_powered: Option<bool>,
    pub power_watts: Option<f64>,
    pub protected: bool,
    pub devices: Vec<Value>,
    pub attachment: Attachment,
    pub configuration: Value,
    pub poe: Option<Value>,
}

pub fn snapshot(switch: &Switch, config: &Config) -> Result<Vec<Port>> {
    let ports = switch.get("swcfg_port", &[("portid", "all")])?;
    let poe = switch.get("swcfg_poe", &[("port", "all")])?;
    let neighbors = switch.neighbors()?;
    let mut data = merge(&ports, &poe, &neighbors, &config.protected_ports)?;
    let vendors = Vendors::load();
    for port in &mut data {
        port.attachment =
            identity::identify(&port.devices, port.link_up, &config.device_names, &vendors);
    }
    Ok(data)
}

fn powered(power: Option<&Value>) -> Option<bool> {
    let power = power?;
    if power["enable"].as_bool() == Some(false) {
        return Some(false);
    }
    match power["status"].as_u64() {
        Some(2) => Some(true),  // Delivering power on the observed M4250 API.
        Some(1) => Some(false), // Searching, not delivering.
        Some(_) => None,        // Fault/other firmware states must not look like "off".
        None => power["currentPower"]
            .as_f64()
            .filter(|v| *v >= 0.0)
            .map(|v| v > 0.0),
    }
}

pub fn merge(
    ports: &Value,
    poe: &Value,
    neighbors: &[Value],
    protected: &[u64],
) -> Result<Vec<Port>> {
    let units = ports
        .pointer("/switchPortConfig/unit")
        .and_then(Value::as_array)
        .context("missing switch units")?;
    let powers = poe
        .get("poePortConfig")
        .and_then(Value::as_array)
        .context("missing PoE configuration")?;
    let mut result = Vec::new();
    for unit in units {
        let unit_id = unit["id"].as_u64().context("missing unit id")?;
        let slots = unit["slot"].as_array().context("missing switch slots")?;
        for slot in slots {
            let slot_id = slot["id"].as_u64().context("missing slot id")?;
            for p in slot["port"].as_array().context("missing switch ports")? {
                let number = p["portNum"].as_u64().context("missing portNum")?;
                let power = powers.iter().find(|v| {
                    v["portNum"].as_u64() == Some(number) && v["unit"].as_u64() == Some(unit_id)
                });
                let link_up = match p["linkState"].as_u64() {
                    Some(0) => Some(true),
                    Some(1) => Some(false),
                    _ => None,
                };
                let devices: Vec<_> = neighbors
                    .iter()
                    .filter(|v| {
                        v["portNum"].as_u64() == Some(number) && v["unit"].as_u64() == Some(unit_id)
                    })
                    .cloned()
                    .collect();
                let attachment =
                    identity::identify(&devices, link_up, &BTreeMap::new(), &Vendors::default());
                result.push(Port {
                    unit: unit_id,
                    slot: slot_id,
                    port: number,
                    link_up,
                    speed: p["linkSpeed"].as_str().unwrap_or("").to_owned(),
                    poe_capable: power.is_some(),
                    poe_enabled: power.and_then(|v| v["enable"].as_bool()),
                    poe_powered: powered(power),
                    power_watts: power
                        .and_then(|v| v["currentPower"].as_f64())
                        .map(|v| v / 1000.0),
                    protected: protected.contains(&number),
                    devices,
                    attachment,
                    configuration: p.clone(),
                    poe: power.cloned(),
                });
            }
        }
    }
    result.sort_by_key(|p| (p.unit, p.slot, p.port));
    Ok(result)
}

pub fn validate_cycle(ports: &[Port], selected: &[u64]) -> Result<()> {
    ensure!(!selected.is_empty(), "select at least one port");
    // The UI reset contract uses portNum only. Refuse ambiguous stacked switches.
    ensure!(
        ports.iter().all(|p| p.unit == 1),
        "PoE reset supported only on standalone unit 1"
    );
    for number in selected {
        let matching: Vec<_> = ports.iter().filter(|p| p.port == *number).collect();
        ensure!(matching.len() == 1, "port {number} absent or ambiguous");
        let p = matching[0];
        ensure!(p.poe_capable, "port {number} does not support PoE");
        ensure!(!p.protected, "port {number} is protected in config");
        ensure!(
            p.poe_enabled == Some(true),
            "port {number} PoE is disabled or unknown"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn power_delivery_is_not_admin_enable_or_link_state() {
        assert_eq!(
            powered(Some(&json!({"enable":true,"status":2,"currentPower":0}))),
            Some(true)
        );
        assert_eq!(
            powered(Some(&json!({"enable":true,"status":1}))),
            Some(false)
        );
        assert_eq!(
            powered(Some(&json!({"enable":false,"status":2}))),
            Some(false)
        );
        assert_eq!(
            powered(Some(&json!({"enable":true,"status":99,"currentPower":0}))),
            None
        );
        assert_eq!(powered(None), None);
    }
    #[test]
    fn merge_keeps_multiple_devices_and_guards_reset() {
        let ports = json!({"switchPortConfig":{"unit":[{"id":1,"slot":[{"id":1,"port":[{"portNum":1,"linkState":0},{"portNum":9,"linkState":1}]}]}]}});
        let poe =
            json!({"poePortConfig":[{"unit":1,"portNum":1,"enable":true,"currentPower":5200}]});
        let neighbors = vec![
            json!({"unit":1,"portNum":1,"source":"lldp"}),
            json!({"unit":1,"portNum":1,"source":"arp"}),
            json!({"unit":2,"portNum":1}),
        ];
        let mut data = merge(&ports, &poe, &neighbors, &[]).unwrap();
        assert_eq!(data[0].devices.len(), 2);
        assert_eq!(data[0].power_watts, Some(5.2));
        assert_eq!(data[0].link_up, Some(true));
        assert!(validate_cycle(&data, &[1]).is_ok());
        assert!(validate_cycle(&data, &[9]).is_err());
        assert!(validate_cycle(&data, &[99]).is_err());
        assert!(validate_cycle(&data, &[]).is_err());
        data[0].protected = true;
        assert!(validate_cycle(&data, &[1]).is_err());
        data[0].protected = false;
        data[0].poe_enabled = Some(false);
        assert!(validate_cycle(&data, &[1]).is_err());
        data[0].unit = 2;
        assert!(validate_cycle(&data, &[1]).is_err());
    }
}
