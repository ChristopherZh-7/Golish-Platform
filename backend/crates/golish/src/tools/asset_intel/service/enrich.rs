//! ASN enrichment helpers (Team Cymru + 0.zone). Moved out of `mod.rs`.

use super::super::*;

pub(crate) async fn lookup_team_cymru_asns(ips: &[IpAddr]) -> Result<Vec<IpAsnMapping>, String> {
    if ips.is_empty() {
        return Ok(Vec::new());
    }
    let timeout = Duration::from_secs(TEAM_CYMRU_ASN_LOOKUP_TIMEOUT_SECS);
    let mut stream = tokio::time::timeout(
        timeout,
        tokio::net::TcpStream::connect(TEAM_CYMRU_WHOIS_ADDR),
    )
    .await
    .map_err(|_| "timed out connecting to Team Cymru whois".to_string())?
    .map_err(|err| format!("connect failed: {err}"))?;
    let query = format!(
        "begin\nverbose\n{}\nend\n",
        ips.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
    tokio::time::timeout(timeout, stream.write_all(query.as_bytes()))
        .await
        .map_err(|_| "timed out writing Team Cymru query".to_string())?
        .map_err(|err| format!("write failed: {err}"))?;
    let mut response = String::new();
    tokio::time::timeout(timeout, stream.read_to_string(&mut response))
        .await
        .map_err(|_| "timed out reading Team Cymru response".to_string())?
        .map_err(|err| format!("read failed: {err}"))?;
    Ok(parse_team_cymru_asn_response(&response))
}

pub(crate) async fn enrich_0zone_asns_from_ip_ranges(
    provider_id: &str,
    run_id: &str,
    profile_entries: &mut Vec<ProfileFieldEntry>,
    sink: Option<&EventEmitterHandle>,
) -> Option<Value> {
    if provider_id != "0.zone"
        || profile_entries
            .iter()
            .any(|entry| entry.target_field == "asns" && !entry.value.trim().is_empty())
    {
        return None;
    }
    let ips = collect_public_ips_for_asn_lookup(profile_entries);
    if ips.is_empty() {
        return None;
    }
    emit_event(
        sink,
        AssetIntelStreamEvent::ProviderProgress {
            run_id: run_id.to_string(),
            provider_id: provider_id.to_string(),
            message: format!("deriving ASN from {} public IP(s)", ips.len()),
            stream: AssetIntelStreamSource::System,
        },
    );
    match lookup_team_cymru_asns(&ips).await {
        Ok(mappings) => {
            let derived = profile_asn_entries_from_mappings(&mappings);
            let asn_count = derived.len();
            profile_entries.extend(derived);
            Some(serde_json::json!({
                "requestId": "team-cymru-ip-to-asn",
                "state": if asn_count == 0 { "checked_empty" } else { "completed" },
                "queriedIpCount": ips.len(),
                "asnCount": asn_count,
            }))
        }
        Err(error) => {
            tracing::warn!(
                provider = %provider_id,
                run_id,
                error,
                "asset_intel derived ASN lookup failed"
            );
            Some(serde_json::json!({
                "requestId": "team-cymru-ip-to-asn",
                "state": "failed",
                "queriedIpCount": ips.len(),
                "error": error,
            }))
        }
    }
}
