//! `#[derive(Ordinal)]`: the `ALL` / `to_u8` / `index` / `from_u8` accessors shared
//! by every small `#[repr(u8)]` enum in `turox-engine` (`Color`, `Piece`,
//! `ColoredPiece`, `File`, `Rank`, `Square`).
//!
//! Hand-rolled token walking, not `syn`: this crate exists so those accessors
//! aren't hand-written six times over, not to pull `syn`/`quote` into the build.
//! `turox-engine` stays free of runtime dependencies either way; this is a
//! compile-time-only proc macro, not a crate its output links against.

use proc_macro::{Delimiter, TokenStream, TokenTree};

/// Derives `ALL`, `to_u8`, `index`, and `from_u8` for a fieldless `#[repr(u8)]`
/// enum whose variants are numbered by declaration order.
///
/// An explicit discriminant is allowed on a variant, but only if it matches
/// that variant's position: `Ordinal`'s generated code always numbers by
/// position, so a mismatched discriminant would silently disagree with the
/// generated methods, the same "looks right, isn't" trap this crate has hit
/// before with hand-written `White`/`Black` or `N`/`S`/`E`/`W` tables. That
/// case is rejected at compile time rather than trusted.
///
/// # Panics
///
/// Never panics: a malformed derive target produces a `compile_error!` in the
/// generated output instead.
#[proc_macro_derive(Ordinal)]
#[must_use]
pub fn derive_ordinal(input: TokenStream) -> TokenStream {
    let Some(name) = enum_name(input.clone()) else {
        return compile_error("Ordinal can only be derived for an enum");
    };
    let Some(body) = enum_body(input) else {
        return compile_error("Ordinal: could not find the enum's variant list");
    };

    match variants_of(&body) {
        Ok(variants) if variants.is_empty() => {
            compile_error("Ordinal: enum must have at least one variant")
        }
        Ok(variants) => generate(&name, &variants),
        Err(message) => compile_error(&message),
    }
}

/// The identifier following the first `enum` keyword in `input`, i.e. the
/// type being derived onto.
fn enum_name(input: TokenStream) -> Option<String> {
    let mut tokens = input.into_iter();
    while let Some(token) = tokens.next() {
        if matches!(&token, TokenTree::Ident(ident) if ident.to_string() == "enum") {
            return match tokens.next() {
                Some(TokenTree::Ident(name)) => Some(name.to_string()),
                _ => None,
            };
        }
    }
    None
}

/// The brace-delimited variant list that follows the enum's name.
fn enum_body(input: TokenStream) -> Option<TokenStream> {
    let mut tokens = input.into_iter();
    for token in tokens.by_ref() {
        if matches!(&token, TokenTree::Ident(ident) if ident.to_string() == "enum") {
            break;
        }
    }
    for token in tokens {
        if let TokenTree::Group(group) = &token {
            if group.delimiter() == Delimiter::Brace {
                return Some(group.stream());
            }
        }
    }
    None
}

/// Splits a variant list on its top-level commas and parses each segment.
/// Returns the variant names in declaration order, or the first parse error
/// encountered.
fn variants_of(body: &TokenStream) -> Result<Vec<String>, String> {
    let mut variants = Vec::new();
    let mut current = Vec::new();
    for token in body.clone() {
        if matches!(&token, TokenTree::Punct(p) if p.as_char() == ',') {
            if !current.is_empty() {
                variants.push(parse_variant(&current, variants.len())?);
                current.clear();
            }
        } else {
            current.push(token);
        }
    }
    if !current.is_empty() {
        variants.push(parse_variant(&current, variants.len())?);
    }
    Ok(variants)
}

/// Parses one comma-separated segment of the variant list: a bare variant
/// name, optionally preceded by attributes and followed by an explicit
/// discriminant. `position` is this variant's declaration-order index, which
/// the discriminant (if present) must match.
fn parse_variant(tokens: &[TokenTree], position: usize) -> Result<String, String> {
    let mut i = 0;
    while i + 1 < tokens.len() {
        let is_attribute = matches!(&tokens[i], TokenTree::Punct(p) if p.as_char() == '#')
            && matches!(&tokens[i + 1], TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket);
        if is_attribute {
            i += 2;
        } else {
            break;
        }
    }

    let name = match tokens.get(i) {
        Some(TokenTree::Ident(ident)) => ident.to_string(),
        _ => return Err("Ordinal: expected a variant name".to_string()),
    };
    i += 1;

    if i == tokens.len() {
        return Ok(name);
    }

    let has_discriminant =
        i + 2 == tokens.len() && matches!(&tokens[i], TokenTree::Punct(p) if p.as_char() == '=');
    if has_discriminant {
        if let TokenTree::Literal(literal) = &tokens[i + 1] {
            if let Ok(value) = literal.to_string().parse::<usize>() {
                if value == position {
                    return Ok(name);
                }
                return Err(format!(
                    "Ordinal: variant `{name}` has explicit discriminant {value}, but its \
                     declaration position is {position}; Ordinal numbers by position, so an \
                     explicit discriminant must agree with it"
                ));
            }
        }
    }

    Err(format!(
        "Ordinal: variant `{name}` must be a plain fieldless variant, optionally with an \
         explicit discriminant matching its declaration position"
    ))
}

/// Builds the `impl` block: `ALL`, `to_u8`, `index`, and `from_u8`.
///
/// `to_u8`/`index` are `self as u8`/`self as usize`, not a per-variant
/// `match`: a first draft used a `match`, on the reasoning that it kept every
/// `as` cast out of the generated code entirely. Benchmarking that draft
/// against `main` (`bitboard_full_iteration`, which pops every set bit via
/// `Square::from_u8`/`Bitboard::pop_lsb` in a tight loop) found a real,
/// statistically significant +15% regression: LLVM doesn't reliably collapse
/// a 64-arm match back down to the trivial bitcast a `#[repr(u8)]` enum's
/// `as u8` already *is*. The cast is both correct (guaranteed by `#[repr(u8)]`
/// on every enum this derives onto) and, empirically, not something worth
/// spending cycles on to avoid.
fn generate(name: &str, variants: &[String]) -> TokenStream {
    let count = variants.len();
    let all = variants
        .iter()
        .map(|v| format!("Self::{v}"))
        .collect::<Vec<_>>()
        .join(", ");

    let code = format!(
        "impl {name} {{
            /// Every variant, in declaration order.
            pub const ALL: [Self; {count}] = [{all}];

            /// This variant's discriminant.
            #[must_use]
            #[allow(clippy::as_conversions)] // see `generate`'s doc in turox-macros
            pub const fn to_u8(self) -> u8 {{
                self as u8
            }}

            /// This variant's discriminant, widened for use as a slice index.
            #[must_use]
            #[allow(clippy::as_conversions)] // see `generate`'s doc in turox-macros
            pub const fn index(self) -> usize {{
                self as usize
            }}

            /// The variant at discriminant `v`, or `None` if `v >= {count}`.
            #[must_use]
            #[allow(clippy::as_conversions)] // v < {count}u8 checked first, so this widening is always in range.
            pub const fn from_u8(v: u8) -> Option<Self> {{
                if v < {count}u8 {{
                    Some(Self::ALL[v as usize])
                }} else {{
                    None
                }}
            }}
        }}"
    );
    code.parse().unwrap_or_else(|_| {
        compile_error("Ordinal: generated code failed to parse (internal error)")
    })
}

/// A `compile_error!(message)` token stream, so a malformed derive target is
/// reported at the derive site rather than panicking the macro itself.
fn compile_error(message: &str) -> TokenStream {
    format!("compile_error!({message:?});")
        .parse()
        .expect("compile_error! invocation is always valid Rust")
}
