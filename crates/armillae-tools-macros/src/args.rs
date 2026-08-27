use syn::{
    Expr, ExprLit, Ident, Lit, Meta, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
};

pub(crate) struct MacroArgs {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) param_descriptions: Vec<(Ident, String)>,
}

impl MacroArgs {
    pub(crate) fn description_for(&self, param: &str) -> Option<&str> {
        self.param_descriptions
            .iter()
            .find(|(ident, _)| ident == param)
            .map(|(_, description)| description.as_str())
    }
}

fn parse_string_literal(expr: &Expr, field_name: &str) -> syn::Result<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(value),
            ..
        }) => Ok(value.value()),
        _ => Err(syn::Error::new_spanned(
            expr,
            format!("`{field_name}` must be a string literal"),
        )),
    }
}

fn validate_tool_name(name: &str, expr: &Expr) -> syn::Result<()> {
    if !(1..=64).contains(&name.chars().count()) {
        return Err(syn::Error::new_spanned(
            expr,
            "`name` must be between 1 and 64 characters long",
        ));
    }

    let mut chars = name.chars();
    if !chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
    {
        return Err(syn::Error::new_spanned(
            expr,
            "`name` must start with an ASCII letter or underscore",
        ));
    }

    if chars
        .any(|character| !character.is_ascii_alphanumeric() && character != '_' && character != '-')
    {
        return Err(syn::Error::new_spanned(
            expr,
            "`name` may only contain ASCII letters, digits, underscores, or hyphens",
        ));
    }

    Ok(())
}

fn reject_duplicate<T>(
    slot: &Option<T>,
    spanned: impl quote::ToTokens,
    name: &str,
) -> syn::Result<()> {
    if slot.is_some() {
        return Err(syn::Error::new_spanned(
            spanned,
            format!("duplicate `{name}` argument"),
        ));
    }
    Ok(())
}

impl Parse for MacroArgs {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut name = None;
        let mut description = None;
        let mut param_descriptions: Option<Vec<(Ident, String)>> = None;
        let arguments = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;

        for argument in arguments {
            match argument {
                Meta::NameValue(name_value) => {
                    let Some(ident) = name_value.path.get_ident() else {
                        return Err(syn::Error::new_spanned(
                            name_value.path,
                            "unsupported top-level #[tool] argument",
                        ));
                    };
                    match ident.to_string().as_str() {
                        "name" => {
                            reject_duplicate(&name, ident, "name")?;
                            let parsed = parse_string_literal(&name_value.value, "name")?;
                            validate_tool_name(&parsed, &name_value.value)?;
                            name = Some(parsed);
                        }
                        "description" => {
                            reject_duplicate(&description, ident, "description")?;
                            description =
                                Some(parse_string_literal(&name_value.value, "description")?);
                        }
                        _ => {
                            return Err(syn::Error::new_spanned(
                                ident,
                                format!("unsupported top-level #[tool] argument `{ident}`"),
                            ));
                        }
                    }
                }
                Meta::List(list) if list.path.is_ident("params") => {
                    reject_duplicate(&param_descriptions, &list.path, "params(...)")?;
                    let descriptions =
                        list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
                    let mut parsed = Vec::new();
                    for description in descriptions {
                        let Meta::NameValue(name_value) = description else {
                            return Err(syn::Error::new_spanned(
                                description,
                                "`params(...)` entries must have the form `name = \"description\"`",
                            ));
                        };
                        let Some(ident) = name_value.path.get_ident().cloned() else {
                            return Err(syn::Error::new_spanned(
                                name_value.path,
                                "parameter descriptions must use identifier keys",
                            ));
                        };
                        if parsed
                            .iter()
                            .any(|(existing, _): &(Ident, String)| existing == &ident)
                        {
                            return Err(syn::Error::new_spanned(
                                &ident,
                                format!("duplicate `params(...)` entry for `{ident}`"),
                            ));
                        }
                        let value = parse_string_literal(&name_value.value, &ident.to_string())?;
                        parsed.push((ident, value));
                    }
                    param_descriptions = Some(parsed);
                }
                unsupported => {
                    return Err(syn::Error::new_spanned(
                        unsupported,
                        "unsupported top-level #[tool] argument",
                    ));
                }
            }
        }

        Ok(Self {
            name,
            description,
            param_descriptions: param_descriptions.unwrap_or_default(),
        })
    }
}
