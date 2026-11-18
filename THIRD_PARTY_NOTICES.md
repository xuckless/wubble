# Third-party notices

Code copied or ported into wubble, with its source and licence. Dependencies pulled
in through Cargo are not listed here.

| What | Where in wubble | Source | Licence |
|---|---|---|---|
| History schema design (urls/visits columns, typed/visit counts); no code copied | `crates/wubble-store/sql/001_init.sql`, `src/history.rs` | Chromium `components/history/core/browser/url_database.cc`, `visit_database.cc` | BSD-3-Clause |
| History/bookmarks schema design and frecency recency buckets (100/70/50/30/10); no code copied | `crates/wubble-store/sql/001_init.sql`, `src/history.rs` | Firefox Places `toolkit/components/places/nsPlacesTables.h` | MPL-2.0 |
