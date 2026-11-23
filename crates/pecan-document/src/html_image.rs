//! Standalone HTML image metadata shared by readers and exports.
/// Parsed metadata of a single standalone HTML image tag.
#[derive(Clone, Debug, PartialEq)]
pub struct HtmlImage {
    /// Image source URI, with HTML entities decoded.
    pub uri: String,
    /// Alternative text, with HTML entities decoded.
    pub alt: String,
    /// Optional positive logical width.
    pub width: Option<f32>,
    /// Optional positive logical height.
    pub height: Option<f32>,
}

/// Accept a single HTML img tag; keep mixed HTML fragments in the existing renderer.
pub fn html_image(source: &str) -> Option<HtmlImage> {
    use html5ever::{
        tendril::StrTendril,
        tokenizer::{BufferQueue, StartTag, Token, TokenSink, TokenSinkResult, Tokenizer},
    };
    use std::cell::{Cell, RefCell};

    // The tokenizer shares its sink, so the sink records what it saw through cells.
    #[derive(Default)]
    struct ImageTag {
        image: RefCell<Option<HtmlImage>>,
        other_content: Cell<bool>,
    }
    impl TokenSink for ImageTag {
        type Handle = ();
        fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
            match token {
                Token::TagToken(tag)
                    if tag.kind == StartTag
                        && &*tag.name == "img"
                        && self.image.borrow().is_none() =>
                {
                    let value = |name: &str| {
                        tag.attrs
                            .iter()
                            .find(|attr| &*attr.name.local == name)
                            .map(|attr| attr.value.to_string())
                    };
                    let dimension = |name: &str| {
                        value(name)
                            .and_then(|value| value.parse::<f32>().ok())
                            .filter(|value| value.is_finite() && *value > 0.)
                    };
                    *self.image.borrow_mut() = value("src").map(|uri| HtmlImage {
                        uri,
                        alt: value("alt").unwrap_or_default(),
                        width: dimension("width"),
                        height: dimension("height"),
                    });
                }
                Token::CharacterTokens(text) if text.trim().is_empty() => {}
                Token::EOFToken | Token::ParseError(_) => {}
                _ => self.other_content.set(true),
            }
            TokenSinkResult::Continue
        }
    }
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(source));
    let tokenizer = Tokenizer::new(ImageTag::default(), Default::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let sink = tokenizer.sink;
    (!sink.other_content.get())
        .then(|| sink.image.into_inner())
        .flatten()
}
