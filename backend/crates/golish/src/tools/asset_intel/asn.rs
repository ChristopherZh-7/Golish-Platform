//! Pure ASN helpers: normalize ASN strings, classify public IPs, and parse
//! Team Cymru WHOIS bulk-lookup responses into profile ASN entries.
//!
//! No IO here: the async WHOIS network lookup lives in the parent module and
//! calls these helpers. Re-exported from the parent module.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::ProfileFieldEntry;

pub(crate) fn normalize_asn(raw: &str) -> String {
    let upper = raw.trim().to_uppercase();
    let digits = upper.strip_prefix("AS").unwrap_or(&upper).trim();
    if digits.is_empty() || digits.len() > 10 || !digits.chars().all(|ch| ch.is_ascii_digit()) {
        return String::new();
    }
    format!("AS{digits}")
}

pub(crate) const TEAM_CYMRU_WHOIS_ADDR: &str = "whois.cymru.com:43";
pub(crate) const TEAM_CYMRU_ASN_LOOKUP_TIMEOUT_SECS: u64 = 8;
const TEAM_CYMRU_ASN_LOOKUP_IP_LIMIT: usize = 40;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IpAsnMapping {
    asn: String,
}

fn parse_ip_for_asn_lookup(raw: &str) -> Option<IpAddr> {
    let without_cidr = raw.trim().split_once('/').map_or(raw.trim(), |(ip, _)| ip);
    without_cidr.parse::<IpAddr>().ok()
}

fn is_public_ipv4_for_asn_lookup(ip: &Ipv4Addr) -> bool {
    let [a, b, c, _d] = ip.octets();
    !matches!(
        (a, b, c),
        (0, _, _)
            | (10, _, _)
            | (100, 64..=127, _)
            | (127, _, _)
            | (169, 254, _)
            | (172, 16..=31, _)
            | (192, 0, 0)
            | (192, 0, 2)
            | (192, 168, _)
            | (198, 18..=19, _)
            | (198, 51, 100)
            | (203, 0, 113)
            | (224..=255, _, _)
    )
}

fn is_public_ipv6_for_asn_lookup(ip: &Ipv6Addr) -> bool {
    let segments = ip.segments();
    let first = segments[0];
    if ip.is_unspecified() || ip.is_loopback() || ip.is_multicast() {
        return false;
    }
    if (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 {
        return false;
    }
    segments[0] != 0x2001 || segments[1] != 0x0db8
}

fn is_public_ip_for_asn_lookup(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4_for_asn_lookup(ip),
        IpAddr::V6(ip) => is_public_ipv6_for_asn_lookup(ip),
    }
}

pub(crate) fn collect_public_ips_for_asn_lookup(entries: &[ProfileFieldEntry]) -> Vec<IpAddr> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if entry.target_kind != golish_pentest::models::AssetIntelProfileFieldTarget::Scalar
            || entry.target_field != "ip_ranges"
        {
            continue;
        }
        let Some(ip) = parse_ip_for_asn_lookup(&entry.value) else {
            continue;
        };
        if !is_public_ip_for_asn_lookup(&ip) || !seen.insert(ip) {
            continue;
        }
        out.push(ip);
        if out.len() >= TEAM_CYMRU_ASN_LOOKUP_IP_LIMIT {
            break;
        }
    }
    out
}

pub(crate) fn parse_team_cymru_asn_response(raw: &str) -> Vec<IpAsnMapping> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for line in raw.lines() {
        let mut cols = line.split('|').map(str::trim);
        let Some(asn_raw) = cols.next() else {
            continue;
        };
        let Some(ip_raw) = cols.next() else {
            continue;
        };
        if asn_raw.eq_ignore_ascii_case("as") {
            continue;
        }
        let asn = normalize_asn(asn_raw);
        let Some(ip) = parse_ip_for_asn_lookup(ip_raw) else {
            continue;
        };
        if asn.is_empty() || !seen.insert((ip, asn.clone())) {
            continue;
        }
        out.push(IpAsnMapping { asn });
    }
    out
}

pub(crate) fn profile_asn_entries_from_mappings(
    mappings: &[IpAsnMapping],
) -> Vec<ProfileFieldEntry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for mapping in mappings {
        if seen.insert(mapping.asn.to_ascii_uppercase()) {
            out.push(ProfileFieldEntry {
                target_kind: golish_pentest::models::AssetIntelProfileFieldTarget::Scalar,
                target_field: "asns".into(),
                value: mapping.asn.clone(),
            });
        }
    }
    out
}
