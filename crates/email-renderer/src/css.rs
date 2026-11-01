//! Narrow tokenized inline-style policy. Unsupported declarations are removed.
//! Stylesheets and resource-loading properties are intentionally not supported.
use cssparser::{Delimiter, ParseError, Parser, ToCss, Token};

fn property_allowed(name: &str) -> bool {
    matches!(
        name,
        "color"
            | "background"
            | "background-color"
            | "font"
            | "font-family"
            | "font-size"
            | "font-weight"
            | "font-style"
            | "line-height"
            | "text-align"
            | "text-decoration"
            | "text-transform"
            | "text-indent"
            | "letter-spacing"
            | "word-spacing"
            | "white-space"
            | "overflow-wrap"
            | "word-break"
            | "vertical-align"
            | "direction"
            | "unicode-bidi"
            | "display"
            | "width"
            | "height"
            | "min-width"
            | "max-width"
            | "min-height"
            | "max-height"
            | "margin"
            | "margin-top"
            | "margin-right"
            | "margin-bottom"
            | "margin-left"
            | "padding"
            | "padding-top"
            | "padding-right"
            | "padding-bottom"
            | "padding-left"
            | "border"
            | "border-width"
            | "border-style"
            | "border-color"
            | "border-top"
            | "border-bottom"
            | "border-left"
            | "border-right"
            | "border-radius"
            | "border-collapse"
            | "border-spacing"
            | "table-layout"
            | "list-style-type"
    )
}
fn value<'i>(p: &mut Parser<'i>, depth: usize) -> Result<String, ParseError<()>> {
    if depth > 8 {
        return Err(ParseError::custom(()));
    }
    let mut out = String::new();
    let mut tokens = 0;
    while !p.is_exhausted() {
        tokens += 1;
        if tokens > 128 {
            return Err(ParseError::custom(()));
        }
        let token = p.next_including_whitespace_and_comments()?.clone();
        match &token {
            Token::Function(name) => {
                if !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "rgb" | "rgba" | "hsl" | "hsla" | "calc" | "min" | "max" | "clamp"
                ) {
                    return Err(ParseError::custom(()));
                }
                let inner = p.parse_nested_block(|p| value(p, depth + 1))?;
                out.push_str(&name.to_ascii_lowercase());
                out.push('(');
                out.push_str(&inner);
                out.push(')');
            }
            Token::UnquotedUrl(_)
            | Token::BadUrl(_)
            | Token::BadString(_)
            | Token::AtKeyword(_)
            | Token::CurlyBracketBlock
            | Token::SquareBracketBlock
            | Token::ParenthesisBlock
            | Token::CloseCurlyBracket
            | Token::CloseSquareBracket
            | Token::CloseParenthesis
            | Token::Semicolon
            | Token::Colon => return Err(ParseError::custom(())),
            Token::Ident(name)
                if matches!(
                    name.to_ascii_lowercase().as_str(),
                    "expression" | "javascript" | "-moz-binding" | "behavior"
                ) =>
            {
                return Err(ParseError::custom(()));
            }
            Token::Number { value, .. } | Token::Dimension { value, .. }
                if !value.is_finite() || value.abs() > 16_384. =>
            {
                return Err(ParseError::custom(()));
            }
            Token::Percentage { unit_value, .. }
                if !unit_value.is_finite() || unit_value.abs() > 10. =>
            {
                return Err(ParseError::custom(()));
            }
            Token::Comment(_) => {}
            _ => out.push_str(&token.to_css_string()),
        }
        if out.len() > 2048 {
            return Err(ParseError::custom(()));
        }
    }
    if out.trim().is_empty() {
        return Err(ParseError::custom(()));
    }
    Ok(out)
}
pub fn sanitize_style(input: &str) -> String {
    if input.len() > 16 * 1024 {
        return String::new();
    }
    let mut p = Parser::new(input);
    let mut result = Vec::new();
    while !p.is_exhausted() && result.len() < 64 {
        let declaration = p.parse_until_before(Delimiter::Semicolon, |p| {
            let property = p.expect_ident_cloned()?.to_ascii_lowercase();
            p.expect_colon()?;
            if !property_allowed(&property) {
                return Err(ParseError::custom(()));
            }
            let body = value(p, 0)?;
            Ok(format!("{property}:{}", body.trim()))
        });
        if let Ok(declaration) = declaration {
            result.push(declaration);
        }
        let _ = p.try_parse(|p| p.expect_semicolon());
    }
    result.join(";")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_layout_and_color() {
        let s =
            sanitize_style("max-width:600px;color:rgb(12, 20, 30);padding:16px;text-align:center");
        assert!(s.contains("max-width:600px"));
        assert!(s.contains("rgb(12, 20, 30)"));
    }
    #[test]
    fn removes_escaped_urls_and_active_css() {
        let s = sanitize_style(
            r#"color:red;background:u\72l(https://bad.test/a);width:expression(alert(1));position:fixed;padding:12px"#,
        );
        assert_eq!(s, "color:red;padding:12px");
    }
    #[test]
    fn does_not_recover_url_tokens_as_safe_values() {
        for css in [
            "background:url('https://bad.test')",
            "background:image-set(url(x) 1x)",
            "font-family:var(--font)",
            "@import 'x';color: red",
        ] {
            assert!(!sanitize_style(css).contains("bad.test"));
        }
    }
    #[test]
    fn encoded_functions_comments_and_nested_values_cannot_fetch_resources() {
        for hostile in [
            r"background:u\000072l(https://tracker.invalid/a)",
            r"background:URL(/**/'https://tracker.invalid/a')",
            r"font-family:var(--remote,url(https://tracker.invalid/a))",
            r"background:rgb(1,2,url(https://tracker.invalid/a))",
            r"width:calc(1px + env(remote))",
            r"background:image-set('https://tracker.invalid/a' 1x)",
            r"background:linear-gradient(red,url(https://tracker.invalid/a))",
            r"width:e\78pression(alert(1))",
        ] {
            assert_eq!(
                sanitize_style(&format!("{hostile};color:blue")),
                "color:blue",
                "{hostile}"
            );
        }
        let deep = format!(
            "width:{}1px{};color:blue",
            "calc(".repeat(256),
            ")".repeat(256)
        );
        assert_eq!(sanitize_style(&deep), "color:blue");
        assert!(sanitize_style(&"color:red;".repeat(2_000)).is_empty());
        assert!(sanitize_style(&"color:red;".repeat(100)).split(';').count() <= 64);
    }
}
