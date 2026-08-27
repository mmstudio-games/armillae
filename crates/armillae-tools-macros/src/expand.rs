use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Attribute, Expr, ExprLit, FnArg, ItemFn, Lit, Meta, Pat, PathArguments, ReturnType, Type,
};

use crate::args::MacroArgs;

struct ModelParam<'a> {
    ident: &'a syn::Ident,
    ty: &'a Type,
    attrs: &'a [Attribute],
    description: Option<String>,
}

pub(crate) fn expand(args: MacroArgs, input: ItemFn) -> syn::Result<TokenStream> {
    validate_signature(&input)?;

    let function_name = &input.sig.ident;
    let tool_name = args
        .name
        .clone()
        .unwrap_or_else(|| function_name.to_string());
    let visibility = &input.vis;
    let tool_type = format_ident!("{}", pascal_case(&function_name.to_string()));
    let arguments_type = format_ident!("__{}Arguments", tool_type);
    let (output_type, error_type) = result_types(&input.sig.output)?;
    let is_async = input.sig.asyncness.is_some();

    let mut model_params = Vec::new();
    let mut call_arguments = Vec::new();
    let mut context_seen = false;
    let mut cleaned_function = input.clone();

    for argument in &input.sig.inputs {
        let FnArg::Typed(parameter) = argument else {
            return Err(syn::Error::new_spanned(
                argument,
                "tool functions cannot have a receiver parameter",
            ));
        };
        let Pat::Ident(pattern) = &*parameter.pat else {
            return Err(syn::Error::new_spanned(
                &parameter.pat,
                "tool parameters must use identifier patterns",
            ));
        };
        if pattern.by_ref.is_some() || pattern.subpat.is_some() {
            return Err(syn::Error::new_spanned(
                pattern,
                "tool parameters must use plain identifier patterns",
            ));
        }

        match parse_parameter_attributes(&parameter.attrs)? {
            ParameterAttributes::Context => {
                if context_seen {
                    return Err(syn::Error::new_spanned(
                        parameter,
                        "a tool function may have at most one `#[tool(context)]` parameter",
                    ));
                }
                if !is_tool_context(&parameter.ty) {
                    return Err(syn::Error::new_spanned(
                        &parameter.ty,
                        "a `#[tool(context)]` parameter must have type `ToolContext`",
                    ));
                }
                context_seen = true;
                call_arguments.push(quote!(context));
            }
            ParameterAttributes::Model { description } => {
                let ident = &pattern.ident;
                model_params.push(ModelParam {
                    ident,
                    ty: &parameter.ty,
                    attrs: &parameter.attrs,
                    description,
                });
                call_arguments.push(quote!(args.#ident));
            }
        }
    }

    for argument in &mut cleaned_function.sig.inputs {
        if let FnArg::Typed(parameter) = argument {
            parameter.attrs.retain(|attribute| {
                !attribute.path().is_ident("doc") && !attribute.path().is_ident("tool")
            });
        }
    }

    let model_names = model_params
        .iter()
        .map(|parameter| parameter.ident.to_string())
        .collect::<Vec<_>>();
    for (ident, _) in &args.param_descriptions {
        if !model_names.iter().any(|name| name == &ident.to_string()) {
            return Err(syn::Error::new_spanned(
                ident,
                format!("unknown tool parameter `{ident}` in `params(...)`"),
            ));
        }
        if model_params
            .iter()
            .any(|parameter| parameter.ident == ident && parameter.description.is_some())
        {
            return Err(syn::Error::new_spanned(
                ident,
                format!(
                    "tool parameter `{ident}` has descriptions in both `params(...)` and `#[tool(description = ...)]`"
                ),
            ));
        }
    }

    let fields = model_params.iter().map(|parameter| {
        let ident = parameter.ident;
        let ty = parameter.ty;
        let description = args
            .description_for(&ident.to_string())
            .map(ToOwned::to_owned)
            .or_else(|| parameter.description.clone())
            .or_else(|| extract_doc_comment(parameter.attrs))
            .unwrap_or_else(|| format!("Parameter {ident}"));
        quote! {
            #[schemars(description = #description)]
            #visibility #ident: #ty
        }
    });

    let description = args
        .description
        .clone()
        .or_else(|| extract_doc_comment(&input.attrs))
        .unwrap_or_else(|| format!("Function to {tool_name}"));
    let await_call = is_async.then(|| quote!(.await));

    Ok(quote! {
        #[derive(
            ::armillae_tools::__private::serde::Deserialize,
            ::armillae_tools::__private::schemars::JsonSchema
        )]
        #[serde(crate = "::armillae_tools::__private::serde")]
        #[schemars(crate = "::armillae_tools::__private::schemars")]
        #[doc(hidden)]
        #visibility struct #arguments_type {
            #(#fields,)*
        }

        #cleaned_function

        #[derive(Clone, Copy, Debug, Default)]
        #visibility struct #tool_type;

        impl ::armillae_tools::Tool for #tool_type {
            type Args = #arguments_type;
            type Output = #output_type;
            type Error = #error_type;

            const NAME: &'static str = #tool_name;

            fn description(&self) -> ::std::borrow::Cow<'static, str> {
                ::std::borrow::Cow::Borrowed(#description)
            }

            async fn call(
                &self,
                context: ::armillae_tools::ToolContext,
                args: Self::Args,
            ) -> Result<Self::Output, Self::Error> {
                #function_name(#(#call_arguments),*) #await_call
            }
        }
    })
}

fn validate_signature(input: &ItemFn) -> syn::Result<()> {
    if input.sig.constness.is_some() {
        return Err(syn::Error::new_spanned(
            input.sig.constness,
            "tool functions cannot be const",
        ));
    }
    if input.sig.unsafety.is_some() {
        return Err(syn::Error::new_spanned(
            input.sig.unsafety,
            "tool functions cannot be unsafe",
        ));
    }
    if input.sig.abi.is_some() {
        return Err(syn::Error::new_spanned(
            &input.sig.abi,
            "tool functions cannot declare an extern ABI",
        ));
    }
    if input.sig.variadic.is_some() {
        return Err(syn::Error::new_spanned(
            &input.sig.variadic,
            "tool functions cannot be variadic",
        ));
    }
    if !input.sig.generics.params.is_empty() || input.sig.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(
            &input.sig.generics,
            "tool functions cannot be generic",
        ));
    }
    Ok(())
}

fn result_types(return_type: &ReturnType) -> syn::Result<(&Type, &Type)> {
    let ReturnType::Type(_, ty) = return_type else {
        return Err(syn::Error::new_spanned(
            return_type,
            "tool functions must return Result<Output, Error>",
        ));
    };
    let Type::Path(type_path) = &**ty else {
        return Err(syn::Error::new_spanned(
            ty,
            "tool functions must return Result<Output, Error>",
        ));
    };
    let Some(result) = type_path.path.segments.last() else {
        return Err(syn::Error::new_spanned(
            type_path,
            "tool functions must return Result<Output, Error>",
        ));
    };
    if result.ident != "Result" {
        return Err(syn::Error::new_spanned(
            &result.ident,
            "tool functions must return Result<Output, Error>",
        ));
    }
    let PathArguments::AngleBracketed(arguments) = &result.arguments else {
        return Err(syn::Error::new_spanned(
            &result.arguments,
            "expected Result<Output, Error>",
        ));
    };
    let mut arguments = arguments.args.iter();
    let Some(syn::GenericArgument::Type(output)) = arguments.next() else {
        return Err(syn::Error::new_spanned(
            &result.arguments,
            "expected Result<Output, Error>",
        ));
    };
    let Some(syn::GenericArgument::Type(error)) = arguments.next() else {
        return Err(syn::Error::new_spanned(
            &result.arguments,
            "expected Result<Output, Error>",
        ));
    };
    if arguments.next().is_some() {
        return Err(syn::Error::new_spanned(
            &result.arguments,
            "expected Result<Output, Error>",
        ));
    }
    if matches!(output, Type::ImplTrait(_)) || matches!(error, Type::ImplTrait(_)) {
        return Err(syn::Error::new_spanned(
            &result.arguments,
            "tool Result types cannot use `impl Trait`",
        ));
    }
    Ok((output, error))
}

enum ParameterAttributes {
    Context,
    Model { description: Option<String> },
}

fn parse_parameter_attributes(attributes: &[Attribute]) -> syn::Result<ParameterAttributes> {
    let mut context = false;
    let mut description = None;
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("tool"))
    {
        let Meta::List(list) = &attribute.meta else {
            return Err(syn::Error::new_spanned(
                attribute,
                "expected `#[tool(context)]` or `#[tool(description = \"...\")]`",
            ));
        };
        let arguments = list.parse_args_with(
            syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated,
        )?;
        for argument in arguments {
            match argument {
                Meta::Path(path) if path.is_ident("context") => {
                    if context {
                        return Err(syn::Error::new_spanned(
                            path,
                            "duplicate `context` parameter marker",
                        ));
                    }
                    context = true;
                }
                Meta::NameValue(name_value) if name_value.path.is_ident("description") => {
                    if description.is_some() {
                        return Err(syn::Error::new_spanned(
                            name_value.path,
                            "duplicate parameter `description` argument",
                        ));
                    }
                    description = Some(parse_parameter_description(&name_value.value)?);
                }
                unsupported => {
                    return Err(syn::Error::new_spanned(
                        unsupported,
                        "parameter `#[tool(...)]` only supports `context` or `description = \"...\"`",
                    ));
                }
            }
        }
    }

    if context && description.is_some() {
        return Err(syn::Error::new_spanned(
            attributes
                .iter()
                .find(|attribute| attribute.path().is_ident("tool"))
                .ok_or_else(|| {
                    syn::Error::new(proc_macro2::Span::call_site(), "missing attribute")
                })?,
            "a ToolContext parameter cannot have a model-facing description",
        ));
    }

    if context {
        Ok(ParameterAttributes::Context)
    } else {
        Ok(ParameterAttributes::Model { description })
    }
}

fn parse_parameter_description(expression: &Expr) -> syn::Result<String> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = expression
    else {
        return Err(syn::Error::new_spanned(
            expression,
            "parameter `description` must be a string literal",
        ));
    };
    Ok(value.value())
}

fn is_tool_context(ty: &Type) -> bool {
    let Type::Path(path) = ty else {
        return false;
    };
    path.path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "ToolContext")
}

fn extract_doc_comment(attributes: &[Attribute]) -> Option<String> {
    let lines = attributes
        .iter()
        .filter_map(|attribute| {
            let Meta::NameValue(name_value) = &attribute.meta else {
                return None;
            };
            if !name_value.path.is_ident("doc") {
                return None;
            }
            let Expr::Lit(ExprLit {
                lit: Lit::Str(value),
                ..
            }) = &name_value.value
            else {
                return None;
            };
            Some(value.value())
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| {
        lines
            .iter()
            .map(|line| line.strip_prefix(' ').unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_owned()
    })
}

fn pascal_case(value: &str) -> String {
    let mut output = String::new();
    let mut capitalize = true;
    for character in value.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            output.extend(character.to_uppercase());
            capitalize = false;
        } else {
            output.push(character);
        }
    }
    output
}
