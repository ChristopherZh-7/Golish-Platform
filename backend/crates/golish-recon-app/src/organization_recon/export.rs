use std::collections::BTreeMap;
use std::io::{Cursor, Write};
use std::path::Path;

use golish_app_core::GolishError;
use zip::write::SimpleFileOptions;

use super::artifacts::write_raw_bytes;
use super::types::{NormalizedReconRecord, ReconArtifactRef, ReconRecordKind};

struct WorkbookSheet {
    name: String,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

pub(crate) fn write_recon_assets_workbook(
    task_dir: &Path,
    records: &[NormalizedReconRecord],
) -> Result<ReconArtifactRef, GolishError> {
    let bytes = build_recon_assets_workbook(records)?;
    write_raw_bytes(
        task_dir,
        Path::new("exports").join("recon-assets.xlsx"),
        &bytes,
        "asset_workbook",
    )
}

pub(crate) fn write_recon_assets_workbook_file(
    output_path: &Path,
    records: &[NormalizedReconRecord],
) -> Result<u64, GolishError> {
    if output_path.as_os_str().is_empty() {
        return Err(GolishError::Validation("output path is empty".into()));
    }
    let bytes = build_recon_assets_workbook(records)?;
    if let Some(parent) = output_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let temp = output_path.with_extension(format!(
        "{}.tmp.{}",
        output_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("xlsx"),
        std::process::id()
    ));
    std::fs::write(&temp, &bytes)?;
    std::fs::rename(temp, output_path)?;
    Ok(bytes.len() as u64)
}

fn build_recon_assets_workbook(records: &[NormalizedReconRecord]) -> Result<Vec<u8>, GolishError> {
    let sheets = workbook_sheets(records);
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut cursor);
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        add_zip_file(
            &mut archive,
            options,
            "[Content_Types].xml",
            content_types_xml(sheets.len()),
        )?;
        add_zip_file(&mut archive, options, "_rels/.rels", root_rels_xml())?;
        add_zip_file(
            &mut archive,
            options,
            "xl/workbook.xml",
            workbook_xml(&sheets),
        )?;
        add_zip_file(
            &mut archive,
            options,
            "xl/_rels/workbook.xml.rels",
            workbook_rels_xml(sheets.len()),
        )?;
        add_zip_file(&mut archive, options, "xl/styles.xml", styles_xml())?;

        for (index, sheet) in sheets.iter().enumerate() {
            add_zip_file(
                &mut archive,
                options,
                &format!("xl/worksheets/sheet{}.xml", index + 1),
                worksheet_xml(sheet),
            )?;
        }
        archive.finish()?;
    }
    Ok(cursor.into_inner())
}

fn add_zip_file(
    archive: &mut zip::ZipWriter<&mut Cursor<Vec<u8>>>,
    options: SimpleFileOptions,
    name: &str,
    content: String,
) -> Result<(), GolishError> {
    archive.start_file(name, options)?;
    archive.write_all(content.as_bytes())?;
    Ok(())
}

fn workbook_sheets(records: &[NormalizedReconRecord]) -> Vec<WorkbookSheet> {
    let mut by_kind: BTreeMap<&'static str, Vec<&NormalizedReconRecord>> = BTreeMap::new();
    for record in records {
        by_kind
            .entry(kind_sheet_name(&record.kind))
            .or_default()
            .push(record);
    }

    let mut summary_rows = Vec::new();
    for (kind, records) in &by_kind {
        summary_rows.push(vec![(*kind).to_string(), records.len().to_string()]);
    }
    if summary_rows.is_empty() {
        summary_rows.push(vec!["no_records".into(), "0".into()]);
    }

    let mut sheets = vec![WorkbookSheet {
        name: "Summary".into(),
        headers: vec!["kind".into(), "count".into()],
        rows: summary_rows,
    }];

    for (kind, kind_records) in by_kind {
        let rows = kind_records
            .into_iter()
            .map(|record| {
                vec![
                    record.record_id.clone(),
                    record_kind_label(&record.kind).into(),
                    record.key.clone(),
                    record.value.clone(),
                    stringify_json(&record.attributes),
                    record.evidence.len().to_string(),
                    record
                        .evidence
                        .iter()
                        .map(|item| item.source_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                ]
            })
            .collect();
        sheets.push(WorkbookSheet {
            name: sanitize_sheet_name(kind),
            headers: vec![
                "record_id".into(),
                "kind".into(),
                "key".into(),
                "value".into(),
                "attributes".into(),
                "evidence_count".into(),
                "sources".into(),
            ],
            rows,
        });
    }

    sheets
}

fn stringify_json(value: &serde_json::Value) -> String {
    if value.is_null() {
        String::new()
    } else if let Some(text) = value.as_str() {
        text.to_string()
    } else {
        serde_json::to_string(value).unwrap_or_default()
    }
}

fn sanitize_sheet_name(name: &str) -> String {
    let mut sheet: String = name
        .chars()
        .map(|ch| match ch {
            '[' | ']' | ':' | '*' | '?' | '/' | '\\' => '_',
            _ => ch,
        })
        .collect();
    sheet.truncate(31);
    if sheet.trim().is_empty() {
        "Sheet".into()
    } else {
        sheet
    }
}

fn kind_sheet_name(kind: &ReconRecordKind) -> &'static str {
    match kind {
        ReconRecordKind::Organization => "organizations",
        ReconRecordKind::Domain => "domains",
        ReconRecordKind::Ip => "ips",
        ReconRecordKind::Port => "ports",
        ReconRecordKind::Service => "services",
        ReconRecordKind::Url => "urls",
        ReconRecordKind::Site => "sites",
        ReconRecordKind::App => "apps",
        ReconRecordKind::MiniProgram => "mini_programs",
        ReconRecordKind::Wechat => "wechat",
        ReconRecordKind::Certificate => "certificates",
        ReconRecordKind::Contact => "contacts",
        ReconRecordKind::Leak => "leaks",
    }
}

fn record_kind_label(kind: &ReconRecordKind) -> &'static str {
    match kind {
        ReconRecordKind::Organization => "organization",
        ReconRecordKind::Domain => "domain",
        ReconRecordKind::Ip => "ip",
        ReconRecordKind::Port => "port",
        ReconRecordKind::Service => "service",
        ReconRecordKind::Url => "url",
        ReconRecordKind::Site => "site",
        ReconRecordKind::App => "app",
        ReconRecordKind::MiniProgram => "mini_program",
        ReconRecordKind::Wechat => "wechat",
        ReconRecordKind::Certificate => "certificate",
        ReconRecordKind::Contact => "contact",
        ReconRecordKind::Leak => "leak",
    }
}

fn content_types_xml(sheet_count: usize) -> String {
    let mut overrides = String::new();
    for index in 1..=sheet_count {
        overrides.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
{overrides}
</Types>"#
    )
}

fn root_rels_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#
        .into()
}

fn workbook_xml(sheets: &[WorkbookSheet]) -> String {
    let mut sheet_xml = String::new();
    for (index, sheet) in sheets.iter().enumerate() {
        sheet_xml.push_str(&format!(
            r#"<sheet name="{}" sheetId="{}" r:id="rId{}"/>"#,
            escape_xml_attr(&sheet.name),
            index + 1,
            index + 1
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets>{sheet_xml}</sheets>
</workbook>"#
    )
}

fn workbook_rels_xml(sheet_count: usize) -> String {
    let mut rels = String::new();
    for index in 1..=sheet_count {
        rels.push_str(&format!(
            r#"<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>"#
        ));
    }
    rels.push_str(&format!(
        r#"<Relationship Id="rId{}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
        sheet_count + 1
    ));
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
    )
}

fn styles_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="1"><fill><patternFill patternType="none"/></fill></fills>
<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
</styleSheet>"#
        .into()
}

fn worksheet_xml(sheet: &WorkbookSheet) -> String {
    let mut rows = String::new();
    let mut all_rows = Vec::with_capacity(sheet.rows.len() + 1);
    all_rows.push(sheet.headers.clone());
    all_rows.extend(sheet.rows.clone());

    for (row_index, row) in all_rows.iter().enumerate() {
        let excel_row = row_index + 1;
        rows.push_str(&format!(r#"<row r="{excel_row}">"#));
        for (col_index, value) in row.iter().enumerate() {
            let cell_ref = cell_ref(col_index, excel_row);
            rows.push_str(&format!(
                r#"<c r="{cell_ref}" t="inlineStr"><is><t{}>{}</t></is></c>"#,
                xml_space_attr(value),
                escape_xml_text(value)
            ));
        }
        rows.push_str("</row>");
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<sheetData>{rows}</sheetData>
</worksheet>"#
    )
}

fn cell_ref(mut col_index: usize, row_index: usize) -> String {
    let mut col = String::new();
    loop {
        let rem = col_index % 26;
        col.insert(0, (b'A' + rem as u8) as char);
        if col_index < 26 {
            break;
        }
        col_index = col_index / 26 - 1;
    }
    format!("{col}{row_index}")
}

fn xml_space_attr(value: &str) -> &'static str {
    if value.trim() != value {
        r#" xml:space="preserve""#
    } else {
        ""
    }
}

fn escape_xml_text(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&apos;".into(),
            _ => ch.to_string(),
        })
        .collect()
}

fn escape_xml_attr(value: &str) -> String {
    escape_xml_text(value)
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use serde_json::json;

    use super::*;
    use crate::organization_recon::types::ReconEvidenceRef;

    fn record(kind: ReconRecordKind, value: &str) -> NormalizedReconRecord {
        NormalizedReconRecord {
            record_id: format!("id:{value}"),
            kind,
            key: format!("key:{value}"),
            value: value.into(),
            attributes: json!({ "field": "fixture" }),
            evidence: vec![ReconEvidenceRef {
                source_id: "fixture".into(),
                run_id: "run".into(),
                task_id: "processing".into(),
                raw_artifact_path: "raw/fixture.json".into(),
            }],
        }
    }

    #[test]
    fn workbook_contains_summary_and_kind_sheets() {
        let bytes = build_recon_assets_workbook(&[
            record(ReconRecordKind::Domain, "example.com"),
            record(ReconRecordKind::App, "Example App"),
        ])
        .unwrap();

        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        assert!(archive.by_name("xl/workbook.xml").is_ok());
        assert!(archive.by_name("xl/worksheets/sheet1.xml").is_ok());

        let mut workbook = String::new();
        archive
            .by_name("xl/workbook.xml")
            .unwrap()
            .read_to_string(&mut workbook)
            .unwrap();
        assert!(workbook.contains(r#"name="Summary""#));
        assert!(workbook.contains(r#"name="apps""#));
        assert!(workbook.contains(r#"name="domains""#));
    }

    #[test]
    fn sheet_xml_escapes_cell_values() {
        let bytes =
            build_recon_assets_workbook(&[record(ReconRecordKind::Domain, "a&b.example")]).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut sheet = String::new();
        archive
            .by_name("xl/worksheets/sheet2.xml")
            .unwrap()
            .read_to_string(&mut sheet)
            .unwrap();

        assert!(sheet.contains("a&amp;b.example"));
    }
}
