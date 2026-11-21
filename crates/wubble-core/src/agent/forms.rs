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

/// Refuses `type` before clicking when the element's accessibility role (Chromium's name) says
/// it takes no text, with the tool that does: a native `<select>` (`comboBoxSelect`, its popup
/// or an option) takes `select`; a button that opens a menu or list (`popUpButton`: a
/// `<button aria-haspopup>`, which `select` refuses) is clicked and its choice clicked; a list
/// box may be either.
pub fn type_role_refusal(role: &str) -> Option<&'static str> {
    match role {
        "comboBoxSelect" | "menuListPopup" | "menuListOption" => {
            Some("this element is a <select> dropdown: use the select tool instead of type")
        }
        "popUpButton" => Some(
            "this element is a button that opens a menu or list: click it, then click the \
             choice you want (type takes text fields only)",
        ),
        "listBox" | "listBoxOption" => Some(
            "this element is a list box: use the select tool for a native <select>, or click \
             the option for a custom list (type takes text fields only)",
        ),
        _ => None,
    }
}

/// The element that has focus, through open shadow roots (a closed one stops at its host), as
/// a JS arrow function `d`.
const DEEP_ACTIVE: &str = "const d=()=>{let a=document.activeElement;\
while(a&&a.shadowRoot&&a.shadowRoot.activeElement)a=a.shadowRoot.activeElement;return a};";

/// The page's registry key `type` call `mark` keeps the element focused before its click under.
fn mark_key(mark: u64) -> String {
    format!("Symbol.for('wubble.type.{mark}')")
}

/// Run just before `type`'s click: keeps the element that has focus now (in the page's main
/// world, under a key only this call names) for [`type_focus_script`] to compare with.
pub fn type_mark_script(mark: u64) -> String {
    let key = mark_key(mark);
    format!(
        "(()=>{{{DEEP_ACTIVE}Object.defineProperty(globalThis,{key},\
         {{value:d(),configurable:true}});return true}})()"
    )
}

/// Run after `type`'s click at `(x, y)`: what has focus now, what is at the point (both through
/// open shadow roots), whether they are the same element, one inside the other (across shadow
/// roots too) or a `<label>` and its control, and whether focus changed since
/// [`type_mark_script`] (`null` when the mark is gone: another document).
pub fn type_focus_script(x: f64, y: f64, mark: u64) -> String {
    let key = mark_key(mark);
    format!(
        "(()=>{{{DEEP_ACTIVE}const a=d(),k={key};\
         const marked=Object.prototype.hasOwnProperty.call(globalThis,k);\
         const b=globalThis[k];delete globalThis[k];\
         let h=document.elementFromPoint({x},{y});\
         while(h&&h.shadowRoot){{const i=h.shadowRoot.elementFromPoint({x},{y});if(!i||i===h)break;h=i}}\
         const has=(p,c)=>{{while(c){{if(c===p)return true;c=c.parentNode||c.host}}return false}};\
         const lbl=h&&h.closest&&h.closest('label');\
         const related=!!(a&&h&&(has(a,h)||has(h,a)||(lbl&&lbl.control===a)));\
         const info=el=>el?{{tag:(el.tagName||'').toLowerCase(),\
         type:el.tagName==='INPUT'?(el.type||'text').toLowerCase():'',\
         editable:!!el.isContentEditable,\
         opaque:!!(el.tagName&&el.tagName.includes('-')&&!el.shadowRoot)}}:null;\
         return{{active:info(a),hit:info(h),related,changed:marked?a!==b:null}}}})()"
    )
}

/// Summary of a DOM element returned by [`type_focus_script`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ElementInfo {
    tag: String,
    input_type: String,
    editable: bool,
    /// A custom element whose shadow root the page's script cannot read (closed, or none):
    /// focus inside a closed root shows as its host, so what holds it cannot be seen.
    opaque: bool,
}

impl ElementInfo {
    fn from_value(v: &Value) -> Option<Self> {
        let obj = v.as_object()?;
        let flag = |k: &str| obj.get(k).and_then(Value::as_bool).unwrap_or(false);
        Some(Self {
            tag: obj.get("tag")?.as_str()?.to_owned(),
            input_type: obj
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            editable: flag("editable"),
            opaque: flag("opaque"),
        })
    }

    fn is_select(&self) -> bool {
        matches!(self.tag.as_str(), "select" | "option")
    }

    fn is_file_input(&self) -> bool {
        self.tag == "input" && self.input_type == "file"
    }

    /// A frame or an opaque host: what has focus inside cannot be seen from here, so it is
    /// taken as able to take text.
    fn may_hold_a_field(&self) -> bool {
        self.opaque || matches!(self.tag.as_str(), "iframe" | "frame")
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

/// The page's answer to [`type_focus_script`].
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TypeFocus {
    /// What has focus after the click.
    pub active: Option<ElementInfo>,
    /// What is at the click's point.
    pub hit: Option<ElementInfo>,
    /// `active` and `hit` are the same element, one holds the other, or `hit` is a label of
    /// `active`.
    pub related: bool,
    /// Focus is on another element than before the click (false when that cannot be told).
    pub changed: bool,
}

/// Whether `type` may type into what has focus after its click. Refused: a `<select>` or a file
/// input, nothing editable focused, or focus that stayed where it was before the click on an
/// element unrelated to the one clicked (the case the check exists for: text meant for a custom
/// widget would overwrite an earlier field). Focus that moved to an editable element is taken
/// even when that element is not the one clicked: editors such as Monaco, CodeMirror 5, Ace and
/// Google Docs focus a hidden textarea or an off-screen frame beside what shows, and a field
/// under an overlay may be focused by the page's script.
pub fn classify_type_focus(f: &TypeFocus) -> Result<(), String> {
    let (active, hit) = (f.active.as_ref(), f.hit.as_ref());
    if hit.is_some_and(ElementInfo::is_select) || active.is_some_and(ElementInfo::is_select) {
        return Err(
            "this element is a <select> dropdown: use the select tool instead of type".into(),
        );
    }
    if hit.is_some_and(ElementInfo::is_file_input) || active.is_some_and(ElementInfo::is_file_input)
    {
        return Err("this element is a file input: type cannot choose files".into());
    }
    let Some(active) = active.filter(|a| !matches!(a.tag.as_str(), "body" | "html" | "")) else {
        return Err("clicking the element did not focus an editable field".into());
    };
    if !active.is_editable() && !active.may_hold_a_field() {
        return Err(format!(
            "focused element <{}> is not editable: type only works on text fields",
            active.tag
        ));
    }
    if !f.changed && !f.related {
        return Err(format!(
            "clicking the element did not move focus (it stayed on an earlier <{}> unrelated to \
             the element clicked); refusing to type so that field is not overwritten",
            active.tag
        ));
    }
    Ok(())
}

/// Parses the JSON reply of [`type_focus_script`] and runs [`classify_type_focus`].
pub fn parse_type_focus(json: &str) -> Result<(), String> {
    let v: Value = serde_json::from_str(json)
        .map_err(|_| "could not verify focus before typing".to_owned())?;
    let flag = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    classify_type_focus(&TypeFocus {
        active: v.get("active").and_then(ElementInfo::from_value),
        hit: v.get("hit").and_then(ElementInfo::from_value),
        related: flag("related"),
        changed: flag("changed"),
    })
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
    fn type_refuses_dropdown_roles_up_front_with_the_tool_that_fits() {
        for role in ["comboBoxSelect", "menuListPopup", "menuListOption"] {
            let msg = type_role_refusal(role).expect(role);
            assert!(msg.contains("use the select tool"), "{role}: {msg}");
        }
        // a <button aria-haspopup>: select refuses it, so the advice is a click
        let msg = type_role_refusal("popUpButton").unwrap();
        assert!(
            msg.contains("click it") && !msg.contains("select tool"),
            "{msg}"
        );
        for role in ["listBox", "listBoxOption"] {
            let msg = type_role_refusal(role).expect(role);
            assert!(
                msg.contains("select tool") && msg.contains("click"),
                "{role}: {msg}"
            );
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

    fn el(tag: &str) -> Option<ElementInfo> {
        let (tag, input_type) = tag.split_once(':').unwrap_or((tag, ""));
        Some(ElementInfo {
            tag: tag.into(),
            input_type: input_type.into(),
            ..ElementInfo::default()
        })
    }

    fn focus(
        active: Option<ElementInfo>,
        hit: Option<ElementInfo>,
        related: bool,
        changed: bool,
    ) -> TypeFocus {
        TypeFocus {
            active,
            hit,
            related,
            changed,
        }
    }

    #[test]
    fn type_takes_focus_that_moved_to_an_editable_element_or_stayed_on_the_target() {
        let cases = [
            // a plain input, clicked: focus moved to it
            (
                "plain input",
                focus(el("input:text"), el("input:text"), true, true),
            ),
            // a label for the field, clicked
            (
                "label for",
                focus(el("input:email"), el("label"), true, true),
            ),
            // Monaco, CodeMirror 5, Ace: the visible element focuses a hidden textarea beside it
            (
                "hidden textarea",
                focus(el("textarea"), el("div"), false, true),
            ),
            // Google Docs: an off-screen frame takes the keys
            (
                "off-screen frame",
                focus(el("iframe"), el("canvas"), false, true),
            ),
            // a field under an overlay that focuses it from script
            ("overlay", focus(el("input:text"), el("div"), false, true)),
            // typing again into the field that already has focus
            (
                "same field again",
                focus(el("input:text"), el("input:text"), true, false),
            ),
            // a field inside a closed shadow root: focus shows as its host
            (
                "closed shadow host",
                focus(
                    Some(ElementInfo {
                        opaque: true,
                        ..el("x-field").unwrap()
                    }),
                    Some(ElementInfo {
                        opaque: true,
                        ..el("x-field").unwrap()
                    }),
                    true,
                    true,
                ),
            ),
            // a contenteditable span's paragraph
            (
                "contenteditable",
                focus(
                    Some(ElementInfo {
                        editable: true,
                        ..el("div").unwrap()
                    }),
                    el("span"),
                    true,
                    true,
                ),
            ),
        ];
        for (what, f) in cases {
            assert_eq!(classify_type_focus(&f), Ok(()), "{what}");
        }
    }

    #[test]
    fn type_refuses_focus_left_on_an_earlier_field_and_what_is_not_editable() {
        // the case the check exists for: a custom widget that takes no focus, the Phone field
        // typed into before still focused
        let err =
            classify_type_focus(&focus(el("input:tel"), el("div"), false, false)).unwrap_err();
        assert!(err.contains("stayed on an earlier <input>"), "{err}");
        // the same through a frame or a closed shadow root
        assert!(classify_type_focus(&focus(el("iframe"), el("div"), false, false)).is_err());
        let host = Some(ElementInfo {
            opaque: true,
            ..el("x-field").unwrap()
        });
        assert!(classify_type_focus(&focus(host, el("div"), false, false)).is_err());
        // a custom widget that takes focus itself (`<div tabindex=0>`): not editable
        let err = classify_type_focus(&focus(el("div"), el("div"), true, true)).unwrap_err();
        assert!(err.contains("<div> is not editable"), "{err}");
        for (what, f) in [
            ("button", focus(el("button"), el("button"), true, true)),
            (
                "checkbox",
                focus(el("input:checkbox"), el("input:checkbox"), true, true),
            ),
            ("body", focus(el("body"), el("div"), true, true)),
            ("nothing", focus(None, None, false, true)),
        ] {
            assert!(classify_type_focus(&f).is_err(), "{what}");
        }
        let sel = classify_type_focus(&focus(el("select"), el("select"), true, true)).unwrap_err();
        assert!(sel.contains("select tool"), "{sel}");
        let file = classify_type_focus(&focus(el("input:tel"), el("input:file"), false, true));
        assert!(file.unwrap_err().contains("file input"));
    }

    #[test]
    fn the_page_answer_is_read_with_unknowns_taken_as_unchanged() {
        let moved = json!({"active": {"tag": "textarea", "type": "", "editable": false, "opaque": false},
            "hit": {"tag": "div", "type": "", "editable": false, "opaque": false},
            "related": false, "changed": true});
        assert_eq!(parse_type_focus(&moved.to_string()), Ok(()));
        // the mark is gone (another document): not taken as a move
        let unknown = json!({"active": {"tag": "textarea", "type": "", "editable": false},
            "hit": {"tag": "div", "type": "", "editable": false},
            "related": false, "changed": null});
        assert!(parse_type_focus(&unknown.to_string()).is_err());
        assert!(parse_type_focus("not json").is_err());
    }

    #[test]
    fn the_mark_and_the_check_name_the_same_key() {
        let key = "Symbol.for('wubble.type.42')";
        assert!(type_mark_script(42).contains(key));
        assert!(type_focus_script(1.0, 2.0, 42).contains(key));
        assert!(type_focus_script(1.0, 2.0, 42).contains("elementFromPoint(1,2)"));
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
