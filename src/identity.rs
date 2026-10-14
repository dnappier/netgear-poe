use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

pub fn normalize_mac(mac: &str) -> Option<String> {
    let hex: String = mac
        .chars()
        .filter(|c| !matches!(c, ':' | '-' | '.'))
        .collect();
    if hex.len() != 12 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let hex = hex.to_uppercase();
    let first = u8::from_str_radix(&hex[..2], 16).ok()?;
    if first & 1 != 0 || hex == "000000000000" {
        return None;
    }
    Some(
        (0..6)
            .map(|i| &hex[i * 2..i * 2 + 2])
            .collect::<Vec<_>>()
            .join(":"),
    )
}

#[derive(Default)]
pub struct Vendors(BTreeMap<String, String>);

impl Vendors {
    pub fn load() -> Self {
        // Offline lookup only: no device MACs sent to an external lookup service.
        fs::read_to_string("/usr/share/hwdata/oui.txt")
            .map(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    fn parse(text: &str) -> Self {
        let mut vendors = BTreeMap::new();
        for line in text.lines() {
            if let Some((prefix, name)) = line.split_once("(base 16)") {
                let prefix = prefix.trim().to_uppercase();
                if prefix.len() == 6 && prefix.bytes().all(|c| c.is_ascii_hexdigit()) {
                    vendors.insert(prefix, name.trim().to_owned());
                }
            }
        }
        Self(vendors)
    }

    fn lookup(&self, mac: &str) -> Option<String> {
        // Locally administered MACs do not have meaningful manufacturer OUIs.
        if u8::from_str_radix(&mac[..2], 16).ok()? & 2 != 0 {
            return None;
        }
        self.0.get(&mac.replace(':', "")[..6]).cloned()
    }
}

#[derive(Serialize)]
pub struct Device {
    pub mac: String,
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub description: Option<String>,
    pub ips: BTreeSet<String>,
    pub sources: BTreeSet<String>,
}

#[derive(Serialize)]
pub struct Attachment {
    /// Neighbor evidence, never an unconditional claim of physical attachment.
    pub status: &'static str,
    pub device: Option<Device>,
    pub candidates: Vec<Device>,
    pub learned_mac_count: usize,
    pub explanation: &'static str,
}

pub fn identify(
    records: &[Value],
    link_up: Option<bool>,
    names: &BTreeMap<String, String>,
    vendors: &Vendors,
) -> Attachment {
    let mut devices: BTreeMap<String, Device> = BTreeMap::new();
    for record in records {
        let Some(mac) = record["hostMacAddress"].as_str().and_then(normalize_mac) else {
            continue;
        };
        let source = record["source"].as_str().unwrap_or("unknown");
        let d = devices.entry(mac.clone()).or_insert_with(|| Device {
            vendor: vendors.lookup(&mac),
            name: names.get(&mac).cloned(),
            mac,
            description: None,
            ips: BTreeSet::new(),
            sources: BTreeSet::new(),
        });
        d.sources.insert(source.to_owned());
        if let Some(ip) = record["hostIpAddress"].as_str().filter(|s| !s.is_empty()) {
            d.ips.insert(ip.to_owned());
        }
        // ARP hostName is often an OUI label, not an actual device name/model.
        if source == "lldp" {
            if let Some(desc) = record["systemDescription"]
                .as_str()
                .filter(|s| !s.is_empty())
            {
                d.description = Some(desc.to_owned());
            }
            if !names.contains_key(&d.mac)
                && let Some(name) = record["hostName"].as_str().filter(|s| !s.is_empty())
            {
                d.name = Some(name.to_owned());
            }
        }
    }
    let count = devices.len();
    // NETGEAR's AV UI takes the first neighbor row for each port, not the
    // lexicographically first MAC or a manufacturer-specific guess.
    let ui_preferred = records
        .first()
        .and_then(|row| row["hostMacAddress"].as_str())
        .and_then(normalize_mac);
    let lldp: Vec<_> = devices
        .iter()
        .filter(|(_, d)| d.sources.contains("lldp"))
        .map(|(mac, _)| mac.clone())
        .collect();
    let (status, chosen, explanation) = if link_up != Some(true) {
        (
            "unknown",
            None,
            "Link is down or unknown; learned records may be stale.",
        )
    } else if lldp.len() == 1 {
        (
            "lldp_neighbor",
            Some(lldp[0].clone()),
            "One LLDP MAC identifies the advertised neighbor. Forwarded/stale LLDP can prevent physical-attachment certainty.",
        )
    } else if lldp.len() > 1 {
        (
            if ui_preferred.is_some() {
                "switch_ui_neighbor"
            } else {
                "ambiguous"
            },
            ui_preferred,
            "Multiple LLDP MACs; preferred device matches the first neighbor record used by NETGEAR AV UI. Physical attachment is not independently verified.",
        )
    } else if count == 1 {
        (
            "single_mac_candidate",
            devices.keys().next().cloned(),
            "One learned MAC is a candidate, not proof of direct attachment; a transparent bridge may be present.",
        )
    } else if count > 1 {
        (
            "ambiguous",
            None,
            "Multiple learned MACs without a unique LLDP neighbor; physical device is unknown.",
        )
    } else {
        (
            "unknown",
            None,
            "No usable MAC identity reported by the switch.",
        )
    };
    let device = chosen.and_then(|mac| devices.remove(&mac));
    // Show only relevant candidates, not an entire downstream network in the main UI.
    let candidates = if matches!(status, "ambiguous" | "switch_ui_neighbor") {
        devices
            .into_values()
            .filter(|d| lldp.is_empty() || d.sources.contains("lldp"))
            .collect()
    } else {
        Vec::new()
    };
    Attachment {
        status,
        device,
        candidates,
        learned_mac_count: count,
        explanation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn row(mac: &str, source: &str) -> Value {
        json!({"hostMacAddress":mac,"source":source,"hostName":"vendor label","hostIpAddress":"192.0.2.1"})
    }
    #[test]
    fn mac_and_vendor_are_normalized_without_guessing_private_ouis() {
        assert_eq!(
            normalize_mac("70a7.4100.0001").as_deref(),
            Some("70:A7:41:00:00:01")
        );
        for bad in [
            "invalid",
            "00:00:00:00:00:00",
            "FF:FF:FF:FF:FF:FF",
            "01:00:5E:00:00:01",
        ] {
            assert!(normalize_mac(bad).is_none());
        }
        let vendors = Vendors::parse("70A741 (base 16) Ubiquiti\n02A741 (base 16) Not reliable");
        assert_eq!(
            vendors.lookup("70:A7:41:00:00:01").as_deref(),
            Some("Ubiquiti")
        );
        assert!(vendors.lookup("02:A7:41:00:00:01").is_none());
    }
    #[test]
    fn neighbor_preferred_over_downstream_and_same_mac_deduplicated() {
        let rows = vec![
            row("70:A7:41:00:00:01", "lldp"),
            row("70-a7-41-00-00-01", "arp"),
            row("DC:A6:32:01:02:03", "arp"),
        ];
        let a = identify(&rows, Some(true), &BTreeMap::new(), &Vendors::default());
        assert_eq!(a.status, "lldp_neighbor");
        assert_eq!(a.learned_mac_count, 2);
        assert_eq!(a.device.unwrap().sources.len(), 2);
        assert!(a.candidates.is_empty());
    }
    #[test]
    fn multiple_neighbors_match_switch_ui_order_and_single_arp_is_only_candidate() {
        let rows = vec![
            row("70:A7:41:00:00:01", "lldp"),
            row("DC:A6:32:01:02:03", "lldp"),
        ];
        let a = identify(&rows, Some(true), &BTreeMap::new(), &Vendors::default());
        assert_eq!(a.status, "switch_ui_neighbor");
        assert_eq!(a.device.unwrap().mac, "70:A7:41:00:00:01");
        assert_eq!(a.candidates.len(), 1);
        let rows = vec![row("DC:A6:32:01:02:03", "arp")];
        let a = identify(&rows, Some(true), &BTreeMap::new(), &Vendors::default());
        assert_eq!(a.status, "single_mac_candidate");
        assert!(a.device.unwrap().name.is_none());
        assert_eq!(
            identify(&rows, Some(false), &BTreeMap::new(), &Vendors::default()).status,
            "unknown"
        );
        let names = BTreeMap::from([("DC:A6:32:01:02:03".into(), "Lab Pi".into())]);
        assert_eq!(
            identify(&rows, Some(true), &names, &Vendors::default())
                .device
                .unwrap()
                .name
                .as_deref(),
            Some("Lab Pi")
        );
    }
    #[test]
    fn crestron_is_selected_from_api_order_not_sorted_mac_or_vendor() {
        let mut crestron = row("00:10:7F:00:00:01", "lldp");
        crestron["hostName"] = json!("CRESTRON ELECTRONICS, INC.");
        crestron["systemDescription"] = json!("CEN-SWPOE-16");
        let other = row("00:00:01:02:03:04", "lldp");
        let a = identify(
            &[crestron.clone(), other.clone()],
            Some(true),
            &BTreeMap::new(),
            &Vendors::default(),
        );
        assert_eq!(a.status, "switch_ui_neighbor");
        let device = a.device.unwrap();
        assert_eq!(device.mac, "00:10:7F:00:00:01");
        assert_eq!(device.description.as_deref(), Some("CEN-SWPOE-16"));
        let a = identify(
            &[other, crestron],
            Some(true),
            &BTreeMap::new(),
            &Vendors::default(),
        );
        assert_eq!(a.device.unwrap().mac, "00:00:01:02:03:04");
    }
}
