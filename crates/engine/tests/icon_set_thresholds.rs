//! Strict icon-set boundaries must survive XLSX import, evaluation and repeated saves (#1160).

use std::io::{Cursor, Read, Write};

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_engine::cf::CfCache;
use gridcraft_model::{CfRule, CondFormat, Workbook};
use gridcraft_xlsx::{read_xlsx, write_xlsx};
use serde_json::json;

fn values() -> Workbook {
    let mut wb = Workbook::new();
    for n in 0..=10 {
        wb.sheet_mut(0).unwrap().set_value(CellRef::new(n, 0), Value::Number(n as f64));
    }
    wb
}

/// Add original, synthetic OOXML to a workbook built in code; no vendor fixtures.
fn fixture(kind: &str, low: &str, high: &str, operators: [&str; 2], reverse: bool) -> Vec<u8> {
    let bytes = write_xlsx(&values()).unwrap();
    let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut output = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for i in 0..input.len() {
        let mut entry = input.by_index(i).unwrap();
        let name = entry.name().to_owned();
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            let cf = format!(
                r#"<conditionalFormatting sqref="A1:A11"><cfRule type="iconSet" priority="1"><iconSet iconSet="3TrafficLights1" reverse="{}" showValue="0"><cfvo type="percent" val="0"/><cfvo type="{kind}" val="{low}"{}/><cfvo type="{kind}" val="{high}"{}/></iconSet></cfRule></conditionalFormatting>"#,
                u8::from(reverse),
                operators[0],
                operators[1]
            );
            data = String::from_utf8(data).unwrap().replace("</worksheet>", &format!("{cf}</worksheet>")).into_bytes();
        }
        output.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        output.write_all(&data).unwrap();
    }
    output.finish().unwrap().into_inner()
}

fn icons(wb: &Workbook) -> Vec<usize> {
    let mut cache = CfCache::new();
    (0..=10)
        .map(|n| {
            let look = cache.look(wb, 0, CellRef::new(n, 0)).unwrap();
            assert!(look.hide_value);
            look.icon.unwrap().1
        })
        .collect()
}

#[test]
fn strict_icon_thresholds_survive_repeated_xlsx_saves() {
    // Expected icons at 0..=10; the strict number/percent boundaries match #1160's observations.
    for (kind, low, high, inclusive, strict, mixed) in [
        ("num", "3", "6", [0, 0, 0, 1, 1, 1, 2, 2, 2, 2, 2], [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 2], [0, 0, 0, 0, 1, 1, 2, 2, 2, 2, 2]),
        ("percent", "30", "70", [0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2], [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2], [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 2]),
        ("percentile", "30", "70", [0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2], [0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2], [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 2]),
        ("formula", "3", "6", [0, 0, 0, 1, 1, 1, 2, 2, 2, 2, 2], [0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 2], [0, 0, 0, 0, 1, 1, 2, 2, 2, 2, 2]),
    ] {
        for (operators, expected) in [
            (["", ""], inclusive),
            ([r#" gte="1""#, r#" gte="true""#], inclusive),
            ([r#" gte="0""#, r#" gte="false""#], strict),
            ([r#" gte="0""#, ""], mixed),
        ] {
            for reverse in [false, true] {
                let expected = expected.map(|i| if reverse { 2 - i } else { i });
                let mut bytes = fixture(kind, low, high, operators, reverse);
                for save in 0..3 {
                    let (wb, report) = read_xlsx(&bytes).unwrap();
                    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
                    assert_eq!(icons(&wb), expected, "kind={kind}, operators={operators:?}, reverse={reverse}, save={save}");
                    bytes = write_xlsx(&wb).unwrap();
                }
            }
        }
    }
}

#[test]
fn icon_threshold_operators_align_when_the_lower_bound_is_omitted() {
    let mut wb = values();
    let rule: CfRule = serde_json::from_value(json!({"IconSet": {
        "set": "3TrafficLights1", "thresholds": [{"Number": 3}, {"Number": 6}],
        "greater_than": [true, false], "reverse": false, "show_value": false
    }}))
    .unwrap();
    wb.sheet_mut(0).unwrap().cond_formats.push(CondFormat {
        ranges: vec![RangeRef::parse("A1:A11").unwrap()],
        rule,
        priority: 1,
        stop_if_true: false,
    });
    let expected = vec![0, 0, 0, 0, 1, 1, 2, 2, 2, 2, 2];
    assert_eq!(icons(&wb), expected);
    let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
    assert_eq!(icons(&back), expected);
}

#[test]
fn legacy_serialized_icon_sets_keep_inclusive_thresholds() {
    let mut wb = values();
    let rule: CfRule = serde_json::from_value(json!({"IconSet": {
        "set": "3TrafficLights1", "thresholds": [{"Number": 0}, {"Number": 3}, {"Number": 6}],
        "reverse": false, "show_value": false
    }}))
    .unwrap();
    wb.sheet_mut(0).unwrap().cond_formats.push(CondFormat {
        ranges: vec![RangeRef::parse("A1:A11").unwrap()],
        rule,
        priority: 1,
        stop_if_true: false,
    });
    assert_eq!(icons(&wb), [0, 0, 0, 1, 1, 1, 2, 2, 2, 2, 2]);
}

#[test]
fn discarded_thresholds_do_not_change_the_default_comparisons() {
    for thresholds in [json!([]), json!([{"Number": 3}])] {
        let mut wb = values();
        let sh = wb.sheet_mut(0).unwrap();
        sh.set_value(CellRef::new(3, 0), Value::Number(33.0));
        sh.set_value(CellRef::new(6, 0), Value::Number(67.0));
        sh.set_value(CellRef::new(10, 0), Value::Number(100.0));
        let rule: CfRule = serde_json::from_value(json!({"IconSet": {
            "set": "3TrafficLights1", "thresholds": thresholds,
            "greater_than": [true, true, true], "reverse": false, "show_value": false
        }}))
        .unwrap();
        sh.cond_formats.push(CondFormat { ranges: vec![RangeRef::parse("A1:A11").unwrap()], rule, priority: 1, stop_if_true: false });
        let expected = icons(&wb);
        assert_eq!(expected[3], 1);
        assert_eq!(expected[6], 2);
        let (back, _) = read_xlsx(&write_xlsx(&wb).unwrap()).unwrap();
        assert_eq!(icons(&back), expected);
    }
}
