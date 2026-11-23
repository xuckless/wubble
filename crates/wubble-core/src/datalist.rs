//! `<datalist>` suggestions for the focused field. Chromium shows a list under the field as the
//! user types; windowless CEF draws none, so a page script reports the focused field's options
//! and box ([`parse`]), and the host shows the ones that match what is typed ([`rows`]) and
//! sends the pick back ([`pick_message`]). Pure. Nothing runs it yet: the list under the field
//! is not in the kit, so the page script (wubble-helper `datalist.js`, commit 2b5733a) and its
//! frame channel wait for it.
//!
//! Which options show is Chromium's `HTMLInputElement::FilteredDataListOptions`
//! (BSD-3-Clause): everything while the field is empty, else the options whose value or label
//! contains the typed text, ignoring case.

use serde_json::Value;

/// One `<option>`: what it sets, and the label shown beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub value: String,
    pub label: String,
}

/// What the page offers for its focused field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    /// The field's box in the frame's viewport: x, y, width, height, physical pixels.
    pub rect: (i32, i32, i32, i32),
    /// What the field holds.
    pub value: String,
    pub items: Vec<Item>,
}

/// The script's report: `None` when it offers nothing (`{}`) or is not one.
pub fn parse(report: &str) -> Option<Offer> {
    let report: Value = serde_json::from_str(report).ok()?;
    let [x, y, w, h]: [i32; 4] = report
        .get("rect")?
        .as_array()?
        .iter()
        .map(|n| n.as_f64().map(|n| n as i32))
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()?;
    let items = report
        .get("options")?
        .as_array()?
        .iter()
        .filter_map(|o| {
            let [value, label] = o.as_array()?.as_slice() else {
                return None;
            };
            Some(Item {
                value: value.as_str()?.to_owned(),
                label: label.as_str()?.to_owned(),
            })
        })
        .collect();
    Some(Offer {
        rect: (x, y, w, h),
        value: report.get("value")?.as_str()?.to_owned(),
        items,
    })
}

/// The options to list for what the field holds now.
pub fn rows(offer: &Offer) -> Vec<&Item> {
    let typed = offer.value.to_lowercase();
    offer
        .items
        .iter()
        .filter(|i| {
            typed.is_empty()
                || i.value.to_lowercase().contains(&typed)
                || i.label.to_lowercase().contains(&typed)
        })
        .collect()
}

/// The answer that sets the field to `value` (`datalist.js` fires `input` and `change`).
pub fn pick_message(value: &str) -> String {
    serde_json::json!({ "value": value }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(value: &str, options: &[(&str, &str)]) -> Offer {
        Offer {
            rect: (0, 0, 0, 0),
            value: value.into(),
            items: options
                .iter()
                .map(|(v, l)| Item {
                    value: (*v).into(),
                    label: (*l).into(),
                })
                .collect(),
        }
    }

    #[test]
    fn the_typed_text_filters_value_and_label_ignoring_case() {
        let options = [
            ("Apple", "fruit"),
            ("banana", "Yellow Thing"),
            ("cherry", ""),
        ];
        let cases: [(&str, &[&str]); 7] = [
            // nothing typed: all of them
            ("", &["Apple", "banana", "cherry"]),
            // a value, anywhere in it, either case
            ("app", &["Apple"]),
            ("ANA", &["banana"]),
            ("rr", &["cherry"]),
            // a label
            ("yellow", &["banana"]),
            ("fru", &["Apple"]),
            ("zzz", &[]),
        ];
        for (typed, want) in cases {
            let o = offer(typed, &options);
            let got: Vec<&str> = rows(&o).iter().map(|i| i.value.as_str()).collect();
            assert_eq!(got, want, "{typed:?}");
        }
    }

    #[test]
    fn a_report_reads_into_an_offer() {
        let report = r#"{"rect":[10.4,20,300,32],"value":"b","options":[["banana","Yellow"],["x",""],[1,2],["only one"]]}"#;
        let mut want = offer("b", &[("banana", "Yellow"), ("x", "")]);
        want.rect = (10, 20, 300, 32);
        assert_eq!(parse(report), Some(want));
        // nothing offered, and what is not a report
        for none in [
            "{}",
            "",
            "null",
            "[]",
            r#"{"rect":[1,2,3],"value":"","options":[]}"#,
        ] {
            assert_eq!(parse(none), None, "{none:?}");
        }
    }

    #[test]
    fn a_pick_is_quoted() {
        assert_eq!(pick_message(r#"a"b"#), r#"{"value":"a\"b"}"#);
    }
}
