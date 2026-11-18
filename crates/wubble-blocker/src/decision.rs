use adblock::blocker::BlockerResult;

/// What to do with a network request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Block,
    /// Serve this `data:` URL instead of the request. Treat as `Block` for documents.
    Redirect {
        data_url: String,
    },
    /// Load this URL instead (`$removeparam`).
    Rewrite {
        url: String,
    },
}

impl Decision {
    pub(crate) fn from_result(r: BlockerResult) -> Self {
        if r.should_block() {
            match r.redirect {
                Some(data_url) => Self::Redirect { data_url },
                None => Self::Block,
            }
        } else {
            match r.rewritten_url {
                Some(url) => Self::Rewrite { url },
                None => Self::Allow,
            }
        }
    }
}
