/// Resource kind of a request, neutral to any browser engine. The engine lane maps
/// its own types (CEF resource types) onto this once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceType {
    Document,
    Subdocument,
    Stylesheet,
    Script,
    Image,
    Font,
    Object,
    Media,
    Xhr,
    Ping,
    Other,
}

impl ResourceType {
    /// The only mapping to adblock's request-type strings.
    // Ported from brave-core components/brave_shields/content/browser/ad_block_request_type_util.cc
    // (ToAdBlockRequestType), MPL-2.0. Strings are those matched by adblock's cpt_match_type.
    pub(crate) fn filter_type(self) -> &'static str {
        match self {
            Self::Document => "main_frame",
            Self::Subdocument => "sub_frame",
            Self::Stylesheet => "stylesheet",
            Self::Script => "script",
            Self::Image => "image",
            Self::Font => "font",
            Self::Object => "object",
            Self::Media => "media",
            Self::Xhr => "xhr",
            Self::Ping => "ping",
            Self::Other => "other",
        }
    }
}

/// A network request to classify. `source_url` is the top-level frame or initiator URL.
#[derive(Clone, Copy, Debug)]
pub struct NetRequest<'a> {
    pub url: &'a str,
    pub source_url: &'a str,
    pub resource_type: ResourceType,
}

#[cfg(test)]
mod tests {
    use super::ResourceType::*;

    #[test]
    fn every_variant_maps_to_its_filter_string() {
        let table = [
            (Document, "main_frame"),
            (Subdocument, "sub_frame"),
            (Stylesheet, "stylesheet"),
            (Script, "script"),
            (Image, "image"),
            (Font, "font"),
            (Object, "object"),
            (Media, "media"),
            (Xhr, "xhr"),
            (Ping, "ping"),
            (Other, "other"),
        ];
        for (ty, s) in table {
            assert_eq!(ty.filter_type(), s);
        }
    }
}
