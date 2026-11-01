//! Reject unsupported template ownership before constructing the DOM. Templates
//! own detached fragments, outside normal DOM depth iteration, and nested template
//! fragments can recurse during destruction. This is a tokenizer policy check,
//! not an HTML sanitizer; the parsed DOM still goes through Ammonia afterward.
use crate::{Error, Result};
use html5ever::tokenizer::{
    BufferQueue, StartTag, TagToken, Token, TokenSink, TokenSinkResult, Tokenizer,
};
use std::cell::Cell;

#[derive(Default)]
struct Check {
    template: Cell<bool>,
}
impl TokenSink for Check {
    type Handle = ();
    fn process_token(&self, token: Token, _: u64) -> TokenSinkResult<()> {
        if let TagToken(tag) = token
            && tag.kind == StartTag
            && &*tag.name == "template"
        {
            self.template.set(true);
        }
        TokenSinkResult::Continue
    }
}
pub(crate) fn preflight(html: &str) -> Result<()> {
    let input = BufferQueue::default();
    input.push_back(html.into());
    let tokenizer = Tokenizer::new(Check::default(), Default::default());
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    if tokenizer.sink.template.get() {
        return Err(Error::Limit("unsupported template markup"));
    }
    Ok(())
}
