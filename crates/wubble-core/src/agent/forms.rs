//! `type` and `select` on form fields: the page scripts they run and the pure reading of the
//! page's answers (the host sends the scripts and acts on the verdicts).

use serde_json::Value;

/// Maximum number of available options listed in a `select` mismatch error.
pub const SELECT_OPTIONS_CAP: usize = 100;
/// `select`: picks an option on a native `<select>` by exact label, exact value,
/// case-insensitive label, or case-insensitive value, and fires `input` and `change`.
pub const SELECT_OPTION: &str = "function(wanted){\
if(!this||this.tagName!=='SELECT'){\
const tag=this&&this.tagName?this.tagName.toLowerCase():'unknown';\
return{ok:false,error:'element <'+tag+'> is not a native <select> dropdown: use click (or type for a searchable combobox) for custom widgets'};\
}\
if(this.disabled)return{ok:false,error:'this <select> dropdown is disabled'};\
const opts=Array.from(this.options).map(o=>({\
opt:o,\
label:(o.text||o.label||'').trim(),\
altLabel:(o.label||'').trim(),\
value:o.value,\
disabled:!!o.disabled||!!(o.parentElement&&o.parentElement.tagName==='OPTGROUP'&&o.parentElement.disabled)\
}));\
const w=String(wanted),wt=w.trim(),wl=wt.toLowerCase();\
const findIn=list=>\
list.find(o=>o.label===wt||o.altLabel===wt)\
||list.find(o=>o.value===w)\
||list.find(o=>o.label.toLowerCase()===wl||o.altLabel.toLowerCase()===wl)\
||list.find(o=>o.value.toLowerCase()===wl);\
const enabled=opts.filter(o=>!o.disabled);\
const hit=findIn(enabled);\
if(!hit){\
const disabledHit=findIn(opts);\
const available=enabled.map(o=>({label:o.label,value:o.value}));\
if(disabledHit)return{ok:false,error:'option '+JSON.stringify(disabledHit.label)+' is disabled',options:available};\
return{ok:false,error:'no option matches '+JSON.stringify(wanted),options:available};\
}\
this.focus();\
this.value=hit.value;\
hit.opt.selected=true;\
this.dispatchEvent(new Event('input',{bubbles:true}));\
this.dispatchEvent(new Event('change',{bubbles:true}));\
return{ok:true,label:hit.label,value:hit.value};\
}";

/// Refuses `type` before clicking when the accessibility role already identifies a dropdown.
pub fn type_role_refusal(role: &str) -> Option<&'static str> {
    match role {
        "popUpButton" | "comboBoxSelect" | "menuListPopup" | "listBox" => {
            Some("this element is a dropdown: use the select tool instead of type")
        }
        _ => None,
    }
}

/// Asks the page what element is at `(x, y)` and what has focus (piercing shadow roots), plus
/// whether they are the same element, ancestor/descendant (including across shadow roots), or a
/// `<label>` pointing at the focused control.
pub fn type_focus_script(x: f64, y: f64) -> String {
    format!(
        "(()=>{{let a=document.activeElement;\
         while(a&&a.shadowRoot&&a.shadowRoot.activeElement)a=a.shadowRoot.activeElement;\
         let h=document.elementFromPoint({x},{y});\
         while(h&&h.shadowRoot){{const i=h.shadowRoot.elementFromPoint({x},{y});if(!i||i===h)break;h=i}}\
         const has=(p,c)=>{{while(c){{if(c===p)return true;c=c.parentNode||c.host}}return false}};\
         const lbl=h&&h.closest&&h.closest('label');\
         const related=!!(a&&h&&(has(a,h)||has(h,a)||(lbl&&lbl.control===a)));\
         const info=el=>el?{{tag:(el.tagName||'').toLowerCase(),\
         type:el.tagName==='INPUT'?(el.type||'text').toLowerCase():'',\
         editable:!!el.isContentEditable}}:null;\
         return{{active:info(a),hit:info(h),related}}}})()"
    )
}

/// Summary of a DOM element returned by [`type_focus_script`].
#[derive(Debug, PartialEq, Eq)]
pub struct ElementInfo {
    tag: String,
    input_type: String,
    editable: bool,
}

impl ElementInfo {
    fn from_value(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        Some(Self {
            tag: obj.get("tag")?.as_str()?.to_owned(),
            input_type: obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            editable: obj
                .get("editable")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }

    fn is_select(&self) -> bool {
        matches!(self.tag.as_str(), "select" | "option")
    }

    fn is_file_input(&self) -> bool {
        self.tag == "input" && self.input_type == "file"
    }

    fn is_frame(&self) -> bool {
        matches!(self.tag.as_str(), "iframe" | "frame")
    }

    fn is_editable(&self) -> bool {
        if self.editable || self.tag == "textarea" {
            return true;
        }
        self.tag == "input"
            && !matches!(
                self.input_type.as_str(),
                "button"
                    | "submit"
                    | "reset"
                    | "image"
                    | "checkbox"
                    | "radio"
                    | "file"
                    | "hidden"
                    | "range"
                    | "color"
            )
    }
}

/// Decides whether `type` may type into the focused element after clicking the target.
pub fn classify_type_focus(
    active: Option<&ElementInfo>,
    hit: Option<&ElementInfo>,
    related: bool,
) -> Result<(), String> {
    if hit.is_some_and(ElementInfo::is_select) || active.is_some_and(ElementInfo::is_select) {
        return Err(
            "this element is a <select> dropdown: use the select tool instead of type".into(),
        );
    }
    if hit.is_some_and(ElementInfo::is_file_input) || active.is_some_and(ElementInfo::is_file_input)
    {
        return Err(
            "this element is a file input: use the upload_file tool instead of type".into(),
        );
    }
    let Some(active) = active else {
        return Err("clicking the element did not focus an editable field".into());
    };
    if matches!(active.tag.as_str(), "body" | "html" | "") {
        return Err("clicking the element did not focus an editable field".into());
    }
    if active.is_frame() {
        return if related {
            Ok(())
        } else {
            Err("clicking the element did not focus the target frame".into())
        };
    }
    if !active.is_editable() {
        return Err(format!(
            "focused element <{}> is not editable: type only works on text fields",
            active.tag
        ));
    }
    if !related {
        return Err(format!(
            "clicking the element did not move focus to it (focus remained on an unrelated <{}>); \
             refusing to type so another field is not overwritten",
            active.tag
        ));
    }
    Ok(())
}

/// Parses the JSON reply of [`type_focus_script`] and runs [`classify_type_focus`].
pub fn parse_type_focus(json: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(json)
        .map_err(|_| "could not verify focus before typing".to_owned())?;
    let active = v.get("active").and_then(ElementInfo::from_value);
    let hit = v.get("hit").and_then(ElementInfo::from_value);
    let related = v.get("related").and_then(Value::as_bool).unwrap_or(false);
    classify_type_focus(active.as_ref(), hit.as_ref(), related)
}

/// Formats the JSON reply of [`SELECT_OPTION`] into a tool success message or error message.
pub fn format_select_result(v: &Value) -> Result<String, String> {
    if v["ok"] == true {
        let label = v["label"].as_str().unwrap_or_default();
        let value = v["value"].as_str().unwrap_or_default();
        return Ok(format!("selected {label:?} (value={value:?})"));
    }
    let error = v["error"]
        .as_str()
        .unwrap_or("could not select option")
        .to_owned();
    let Some(opts) = v.get("options").and_then(Value::as_array) else {
        return Err(error);
    };
    if opts.is_empty() {
        return Err(format!("{error}; no enabled options available"));
    }
    let mut listed: Vec<String> = opts
        .iter()
        .take(SELECT_OPTIONS_CAP)
        .map(|o| {
            let label = o["label"].as_str().unwrap_or_default();
            let value = o["value"].as_str().unwrap_or_default();
            if value.is_empty() || value == label {
                format!("{label:?}")
            } else {
                format!("{label:?} (value={value:?})")
            }
        })
        .collect();
    if opts.len() > SELECT_OPTIONS_CAP {
        listed.push(format!("... (+{} more)", opts.len() - SELECT_OPTIONS_CAP));
    }
    Err(format!("{error}; available options: {}", listed.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn type_refuses_dropdown_roles_up_front() {
        for role in ["popUpButton", "comboBoxSelect", "menuListPopup", "listBox"] {
            let msg = type_role_refusal(role).expect(role);
            assert!(msg.contains("select"), "{role}: {msg}");
        }
        for role in [
            "textField",
            "searchBox",
            "comboBoxGrouping",
            "button",
            "link",
        ] {
            assert_eq!(type_role_refusal(role), None, "{role}");
        }
    }

    #[test]
    fn type_focus_classification_accepts_editable_targets_and_refuses_others() {
        let ok_cases = [
            json!({"active": {"tag": "input", "type": "text", "editable": false}, "hit": {"tag": "input", "type": "text", "editable": false}, "related": true}),
            json!({"active": {"tag": "input", "type": "email", "editable": false}, "hit": {"tag": "label", "type": "", "editable": false}, "related": true}),
            json!({"active": {"tag": "textarea", "type": "", "editable": false}, "hit": {"tag": "textarea", "type": "", "editable": false}, "related": true}),
            json!({"active": {"tag": "div", "type": "", "editable": true}, "hit": {"tag": "span", "type": "", "editable": true}, "related": true}),
            json!({"active": {"tag": "iframe", "type": "", "editable": false}, "hit": {"tag": "iframe", "type": "", "editable": false}, "related": true}),
        ];
        for case in ok_cases {
            assert!(parse_type_focus(&case.to_string()).is_ok(), "{case}");
        }

        // <select> (either hit or active)
        let sel = json!({"active": {"tag": "select", "type": "", "editable": false}, "hit": {"tag": "select", "type": "", "editable": false}, "related": true});
        assert!(
            parse_type_focus(&sel.to_string())
                .unwrap_err()
                .contains("select tool")
        );

        // <input type="file"> (either hit or active)
        let file = json!({"active": {"tag": "input", "type": "tel", "editable": false}, "hit": {"tag": "input", "type": "file", "editable": false}, "related": false});
        assert!(
            parse_type_focus(&file.to_string())
                .unwrap_err()
                .contains("upload_file")
        );

        // Unrelated previously focused text input (e.g. Phone field) when clicking a custom widget
        let unrelated = json!({"active": {"tag": "input", "type": "tel", "editable": false}, "hit": {"tag": "div", "type": "", "editable": false}, "related": false});
        let err = parse_type_focus(&unrelated.to_string()).unwrap_err();
        assert!(err.contains("unrelated"), "{err}");

        // Non-editable focused elements
        for bad in [
            json!({"active": {"tag": "button", "type": "", "editable": false}, "hit": {"tag": "button", "type": "", "editable": false}, "related": true}),
            json!({"active": {"tag": "input", "type": "checkbox", "editable": false}, "hit": {"tag": "input", "type": "checkbox", "editable": false}, "related": true}),
            json!({"active": {"tag": "body", "type": "", "editable": false}, "hit": {"tag": "div", "type": "", "editable": false}, "related": true}),
            json!({"active": null, "hit": null, "related": false}),
            json!({"active": {"tag": "iframe", "type": "", "editable": false}, "hit": {"tag": "div", "type": "", "editable": false}, "related": false}),
        ] {
            assert!(parse_type_focus(&bad.to_string()).is_err(), "{bad}");
        }
        assert!(parse_type_focus("not json").is_err());
    }

    #[test]
    fn select_result_formatting() {
        let ok = json!({"ok": true, "label": "Toronto, ON", "value": "yyz"});
        assert_eq!(
            format_select_result(&ok).unwrap(),
            "selected \"Toronto, ON\" (value=\"yyz\")"
        );

        let miss = json!({
            "ok": false,
            "error": "no option matches \"NYC\"",
            "options": [
                {"label": "Toronto, ON", "value": "yyz"},
                {"label": "Montreal", "value": "Montreal"}
            ]
        });
        let err = format_select_result(&miss).unwrap_err();
        assert!(err.contains("no option matches \"NYC\""), "{err}");
        assert!(err.contains("\"Toronto, ON\" (value=\"yyz\")"), "{err}");
        assert!(err.contains("\"Montreal\""), "{err}");
        assert!(!err.contains("value=\"Montreal\""), "{err}");

        let empty = json!({"ok": false, "error": "no option matches \"x\"", "options": []});
        assert!(
            format_select_result(&empty)
                .unwrap_err()
                .contains("no enabled options")
        );

        let non_select = json!({
            "ok": false,
            "error": "element <div> is not a native <select> dropdown: use click (or type for a searchable combobox) for custom widgets"
        });
        assert!(
            format_select_result(&non_select)
                .unwrap_err()
                .contains("not a native <select>")
        );
    }
}
