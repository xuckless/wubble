//! `type` and `select` on form fields: the page scripts they run and the pure reading of the
//! page's answers (the host sends the scripts and acts on the verdicts).

use serde_json::Value;

/// Characters of one option's label or value a `select` reply carries (page text, so a
/// prompt-injection surface: kept short).
pub const LABEL_CHARS: usize = 80;
/// Options a `select` mismatch lists at most (the reply's size cap cuts it sooner).
pub const SELECT_OPTIONS_CAP: usize = 100;
/// Bytes of a `select` reply at most.
pub const REPLY_BYTES: usize = 4096;
/// Characters of what the page's script threw that a reply carries.
pub const THROWN_CHARS: usize = 200;

/// `select`, called on the ref's node (a `<select>`, or an `<option>`: its own `<select>`, that
/// option alone): picks the enabled option whose label (trimmed), then value, then label with
/// Unicode spaces made plain, then that without case, then value without case equals `wanted`;
/// the first of equal ones. Sets it as the list's only choice and fires `input` and `change`.
/// Answers with codes, not prose: the host words them ([`format_select_result`]), page text
/// cut to [`LABEL_CHARS`] here and again there. Sent as [`select_option_script`].
const SELECT_OPTION: &str = "function(wanted){\
const cut=s=>{s=String(s);return s.length>LABEL_CHARS?s.slice(0,LABEL_CHARS):s};\
let sel=this,only=null;\
if(sel&&sel.tagName==='OPTION'){only=sel;sel=sel.closest('select')}\
if(!sel||sel.tagName!=='SELECT')\
return{ok:false,why:'not-select',tag:this&&this.tagName?cut(this.tagName.toLowerCase()):'unknown'};\
if(sel.matches(':disabled'))return{ok:false,why:'disabled'};\
const norm=s=>String(s).replace(/[\\s\\u200b]+/g,' ').trim(),low=s=>norm(s).toLowerCase();\
let opts=Array.from(sel.options).map(o=>({opt:o,label:(o.text||o.label||'').trim(),\
alt:(o.label||'').trim(),value:o.value,disabled:o.matches(':disabled')}));\
if(only)opts=opts.filter(o=>o.opt===only);\
const W=String(wanted),wt=W.trim(),wn=norm(W),wl=wn.toLowerCase();\
const tiers=[o=>o.label===wt||o.alt===wt,o=>o.value===W,\
o=>norm(o.label)===wn||norm(o.alt)===wn,o=>low(o.label)===wl||low(o.alt)===wl,\
o=>low(o.value)===wl];\
const find=list=>{for(const t of tiers){const m=list.filter(t);if(m.length)return m}return[]};\
const enabled=opts.filter(o=>!o.disabled);\
const listed={options:enabled.slice(0,OPTIONS_CAP).map(o=>({label:cut(o.label),value:cut(o.value)})),\
total:enabled.length};\
const m=find(enabled);\
if(!m.length){\
const d=find(opts);\
if(d.length)return{ok:false,why:'option-disabled',label:cut(d[0].label),...listed};\
if(only)return{ok:false,why:'not-this-option',label:cut(only.text||only.label||'')};\
return{ok:false,why:'no-match',...listed};\
}\
const hit=m[0];\
sel.focus();\
for(const o of sel.options)o.selected=o===hit.opt;\
sel.dispatchEvent(new Event('input',{bubbles:true}));\
sel.dispatchEvent(new Event('change',{bubbles:true}));\
return{ok:true,label:cut(hit.label),value:cut(hit.value),matches:m.length,multiple:!!sel.multiple};\
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

/// [`SELECT_OPTION`] with its caps written in.
pub fn select_option_script() -> String {
    SELECT_OPTION
        .replace("LABEL_CHARS", &LABEL_CHARS.to_string())
        .replace("OPTIONS_CAP", &SELECT_OPTIONS_CAP.to_string())
}

/// `text` cut to `max` characters, an ellipsis saying so.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

/// `text` cut to [`REPLY_BYTES`] (at a character's edge).
fn capped(mut text: String) -> String {
    if text.len() > REPLY_BYTES {
        let mut end = REPLY_BYTES - '…'.len_utf8();
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
        text.push('…');
    }
    text
}

/// A page string of a [`SELECT_OPTION`] answer, quoted and cut.
fn page_text(v: &Value) -> String {
    format!("{:?}", cut(v.as_str().unwrap_or_default(), LABEL_CHARS))
}

/// The enabled options a [`SELECT_OPTION`] answer lists, as many as fit the reply.
fn options_listed(v: &Value) -> String {
    let opts = v["options"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let total = v["total"].as_u64().map_or(opts.len(), |t| t as usize);
    if total == 0 {
        return "no enabled options available".into();
    }
    let mut out = String::from("available options: ");
    let mut shown = 0;
    for (i, o) in opts.iter().take(SELECT_OPTIONS_CAP).enumerate() {
        let (label, value) = (page_text(&o["label"]), page_text(&o["value"]));
        let one = if value == "\"\"" || value == label {
            label
        } else {
            format!("{label} (value={value})")
        };
        // room left for the note of how many more
        if out.len() + one.len() + 40 > REPLY_BYTES - 200 {
            break;
        }
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&one);
        shown += 1;
    }
    if total > shown {
        out.push_str(&format!(", ... (+{} more)", total - shown));
    }
    out
}

/// Words the answer of [`SELECT_OPTION`] for `wanted` (the agent's own `option`): the success
/// message, or the error. Page text in it is cut to [`LABEL_CHARS`] a string, the whole to
/// [`REPLY_BYTES`].
pub fn format_select_result(v: &Value, wanted: &str) -> Result<String, String> {
    let wanted = format!("{:?}", cut(wanted, LABEL_CHARS));
    if v["ok"] == true {
        let mut out = format!(
            "selected {} (value={})",
            page_text(&v["label"]),
            page_text(&v["value"])
        );
        if let Some(n) = v["matches"].as_u64().filter(|n| *n > 1) {
            out.push_str(&format!(
                "; {n} options match {wanted} equally: the first was chosen (name another by \
                 its value)"
            ));
        }
        if v["multiple"] == true {
            out.push_str("; this list takes several choices: select made this its only one");
        }
        return Ok(capped(out));
    }
    let error = match v["why"].as_str() {
        Some("not-select") => format!(
            "element <{}> is not a native <select> dropdown: for a custom dropdown, click it, \
             then click the choice (or type into a searchable combobox)",
            cut(v["tag"].as_str().unwrap_or("unknown"), 40)
        ),
        Some("disabled") => "this <select> dropdown is disabled".into(),
        Some("option-disabled") => format!(
            "option {} is disabled; {}",
            page_text(&v["label"]),
            options_listed(v)
        ),
        Some("not-this-option") => format!(
            "the ref is the option {}, which {wanted} does not name: name it, or pass the \
             <select>'s ref",
            page_text(&v["label"])
        ),
        Some("no-match") => format!("no option matches {wanted}; {}", options_listed(v)),
        _ => "could not select option: the page's answer was not understood".into(),
    };
    Err(capped(error))
}

/// The error for a `select` whose script threw (`exceptionDetails`): what it threw, cut to
/// [`THROWN_CHARS`] (the page chooses that text).
pub fn select_threw(details: &Value) -> String {
    let what = details["exception"]["description"]
        .as_str()
        .or_else(|| details["text"].as_str())
        .unwrap_or("an exception");
    format!(
        "could not select option: the page's script threw: {}",
        cut(what, THROWN_CHARS)
    )
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
        let ok = json!({"ok": true, "label": "Toronto, ON", "value": "yyz", "matches": 1, "multiple": false});
        assert_eq!(
            format_select_result(&ok, "Toronto, ON").unwrap(),
            "selected \"Toronto, ON\" (value=\"yyz\")"
        );
        // equal labels: the first, and the reply says so
        let dup = json!({"ok": true, "label": "Paris", "value": "fr", "matches": 2});
        let msg = format_select_result(&dup, "Paris").unwrap();
        assert!(
            msg.contains("2 options match \"Paris\" equally: the first"),
            "{msg}"
        );
        // <select multiple>: one choice, said
        let many =
            json!({"ok": true, "label": "Red", "value": "r", "matches": 1, "multiple": true});
        let msg = format_select_result(&many, "Red").unwrap();
        assert!(msg.contains("its only one"), "{msg}");

        let miss = json!({
            "ok": false,
            "why": "no-match",
            "options": [
                {"label": "Toronto, ON", "value": "yyz"},
                {"label": "Montreal", "value": "Montreal"}
            ],
            "total": 2
        });
        let err = format_select_result(&miss, "NYC").unwrap_err();
        assert!(err.contains("no option matches \"NYC\""), "{err}");
        assert!(err.contains("\"Toronto, ON\" (value=\"yyz\")"), "{err}");
        assert!(err.contains("\"Montreal\""), "{err}");
        assert!(!err.contains("value=\"Montreal\""), "{err}");

        let empty = json!({"ok": false, "why": "no-match", "options": [], "total": 0});
        let err = format_select_result(&empty, "x").unwrap_err();
        assert!(err.contains("no enabled options"), "{err}");

        let non_select = json!({"ok": false, "why": "not-select", "tag": "button"});
        let err = format_select_result(&non_select, "x").unwrap_err();
        assert!(
            err.contains("<button> is not a native <select>") && err.contains("click it"),
            "{err}"
        );

        let off = json!({"ok": false, "why": "option-disabled", "label": "Sold out",
            "options": [{"label": "S", "value": "s"}], "total": 1});
        let err = format_select_result(&off, "sold out").unwrap_err();
        assert!(
            err.contains("option \"Sold out\" is disabled; available options: \"S\""),
            "{err}"
        );
        let other = json!({"ok": false, "why": "not-this-option", "label": "Canada"});
        let err = format_select_result(&other, "Mexico").unwrap_err();
        assert!(err.contains("the ref is the option \"Canada\""), "{err}");
        assert!(format_select_result(&json!("nonsense"), "x").is_err());
    }

    #[test]
    fn page_text_in_select_replies_is_capped() {
        let long = "A".repeat(10_000);
        let ok = json!({"ok": true, "label": long, "value": long});
        let msg = format_select_result(&ok, "x").unwrap();
        assert!(msg.chars().count() < 2 * LABEL_CHARS + 40, "{}", msg.len());
        // a thousand long options: the error stays under the cap, saying how many are left out
        let options: Vec<Value> = (0..1000)
            .map(|i| json!({"label": format!("{i}{long}"), "value": format!("v{i}{long}")}))
            .collect();
        let miss = json!({"ok": false, "why": "no-match", "options": options, "total": 1000});
        let err = format_select_result(&miss, &long).unwrap_err();
        assert!(err.len() <= REPLY_BYTES, "{}", err.len());
        assert!(err.contains("more)"), "{err}");
        // a script that throws a megabyte
        let thrown = json!({"exception": {"description": "E".repeat(1 << 20)}});
        let err = select_threw(&thrown);
        assert!(err.chars().count() < THROWN_CHARS + 60, "{}", err.len());
        // multi-byte text is cut at a character's edge
        let wide = json!({"ok": false, "why": "no-match",
            "options": (0..200).map(|i| json!({"label": format!("{i}{}", "é".repeat(79)), "value": ""})).collect::<Vec<_>>(),
            "total": 200});
        assert!(format_select_result(&wide, "x").unwrap_err().len() <= REPLY_BYTES);
    }

    #[test]
    fn the_select_script_takes_its_caps_from_here() {
        let script = select_option_script();
        assert!(script.contains(&format!("s.length>{LABEL_CHARS}?s.slice(0,{LABEL_CHARS})")));
        assert!(script.contains(&format!("slice(0,{SELECT_OPTIONS_CAP})")));
        assert!(!script.contains("LABEL_CHARS") && !script.contains("OPTIONS_CAP"));
        // the Unicode-space class reaches the page as a regex escape
        assert!(script.contains(r"/[\s\u200b]+/g"));
    }
}
