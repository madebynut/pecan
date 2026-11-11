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

    #[derive(Default)]
    struct ImageTag {
        image: Option<HtmlImage>,
        other_content: bool,
    }
    impl TokenSink for ImageTag {
        type Handle = ();
        fn process_token(&mut self, token: Token, _: u64) -> TokenSinkResult<()> {
            match token {
                Token::TagToken(tag)
                    if tag.kind == StartTag
                        && tag.name.as_ref() == "img"
                        && self.image.is_none() =>
                {
                    let uri = tag
                        .attrs
                        .iter()
                        .find(|attr| attr.name.local.as_ref() == "src")
                        .map(|attr| attr.value.to_string());
                    let alt = tag
                        .attrs
                        .iter()
                        .find(|attr| attr.name.local.as_ref() == "alt")
                        .map_or_else(String::new, |attr| attr.value.to_string());
                    let dimension = |name: &str| {
                        tag.attrs
                            .iter()
                            .find(|attr| attr.name.local.as_ref() == name)
                            .and_then(|attr| attr.value.parse::<f32>().ok())
                            .filter(|value| value.is_finite() && *value > 0.)
                    };
                    self.image = uri.map(|uri| HtmlImage {
                        uri,
                        alt,
                        width: dimension("width"),
                        height: dimension("height"),
                    });
                }
                Token::CharacterTokens(text) if text.trim().is_empty() => {}
                Token::EOFToken | Token::ParseError(_) => {}
                _ => self.other_content = true,
            }
            TokenSinkResult::Continue
        }
    }
    let mut input = BufferQueue::default();
    input.push_back(StrTendril::from(source));
    let mut tokenizer = Tokenizer::new(ImageTag::default(), Default::default());
    let _ = tokenizer.feed(&mut input);
    tokenizer.end();
    (!tokenizer.sink.other_content)
        .then_some(tokenizer.sink.image)
        .flatten()
}
