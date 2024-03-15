// Copyright 2019-2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

//! Forest wishes to provide [OpenRPC](http://open-rpc.org) definitions for
//! Filecoin APIs.
//! To do this, it needs:
//! - [JSON Schema](https://json-schema.org/) definitions for all the argument
//!   and return types.
//! - The number of arguments ([arity](https://en.wikipedia.org/wiki/Arity)) and
//!   names of those arguments for each RPC method.
//!
//! We have all this information in our handler functions, we just need to use it.
//! - [`schemars::JsonSchema`] provides schema definitions, with our [`GenerateSchemas`]
//!   trait acting as a shim.
//! - [`Wrap`] defining arity and actually dispatching the function calls.
//!
//! [`SelfDescribingModule`] actually does the work to create the OpenRPC document.

pub mod jsonrpc_types;
pub mod openrpc_types;

mod parser;
mod util;

use std::{
    future::{ready, Future, Ready},
    pin::Pin,
    sync::Arc,
    task::{ready, Context, Poll},
};

use futures::future::Either;
use itertools::Itertools as _;
use openrpc_types::{ContentDescriptor, Method, ParamStructure};
use parser::Parser;
use pin_project_lite::pin_project;
use schemars::{
    gen::{SchemaGenerator, SchemaSettings},
    schema::Schema,
    JsonSchema,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::error::JsonRpcError;
use util::Optional as _;

/// A wrapped [`jsonrpsee::server::RpcModule`] which generates an [OpenRPC](https://spec.open-rpc.org)
/// schema for the methods.
pub struct SelfDescribingModule<Ctx> {
    inner: jsonrpsee::server::RpcModule<Ctx>,
    schema_generator: SchemaGenerator,
    calling_convention: ParamStructure,
    methods: Vec<Method>,
}

impl<Ctx> SelfDescribingModule<Ctx> {
    pub fn new(ctx: Ctx, calling_convention: ParamStructure) -> Self {
        Self {
            inner: jsonrpsee::server::RpcModule::new(ctx),
            schema_generator: SchemaGenerator::new(SchemaSettings::openapi3()),
            calling_convention,
            methods: vec![],
        }
    }
    /// Even if the calling convention is [`ParamStructure::ByPosition`], OpenRPC
    /// requires each argument to have a unique name.
    ///
    /// # Panics
    /// - if param names aren't unique
    /// - if optional parameters follow mandatory params
    pub fn serve<const ARITY: usize, F, Args, R>(
        &mut self,
        method_name: &'static str,
        param_names: [&'static str; ARITY],
        f: F,
    ) -> &mut Self
    where
        F: Wrap<ARITY, Ctx, Args, R>,
        Ctx: Send + Sync + 'static,
        Args: GenerateSchemas,
        R: JsonSchema + for<'de> Deserialize<'de>,
    {
        self.inner
            .register_async_method(method_name, f.wrap(param_names, self.calling_convention))
            .unwrap();
        self.methods.push(Method {
            name: String::from(method_name),
            params: openrpc_types::Params::new(
                Args::generate_schemas(&mut self.schema_generator)
                    .into_iter()
                    .zip_eq(param_names)
                    .map(|((schema, optional), name)| ContentDescriptor {
                        name: String::from(name),
                        schema,
                        required: !optional,
                    }),
            )
            .unwrap(),
            param_structure: self.calling_convention,
            result: Some(ContentDescriptor {
                name: format!("{}-result", method_name),
                schema: R::json_schema(&mut self.schema_generator),
                required: !R::optional(),
            }),
        });
        self
    }
    pub fn finish(self) -> (jsonrpsee::server::RpcModule<Ctx>, openrpc_types::OpenRPC) {
        let Self {
            inner,
            mut schema_generator,
            methods,
            calling_convention: _,
        } = self;
        (
            inner,
            openrpc_types::OpenRPC {
                methods: openrpc_types::Methods::new(methods).unwrap(),
                components: openrpc_types::Components {
                    schemas: schema_generator.take_definitions().into_iter().collect(),
                },
            },
        )
    }
}

/// Wrap a bare function with our argument parsing logic.
/// Turns any `async fn foo(ctx, arg0...)` into a function that can be passed to [`jsonrpsee::server::RpcModule::register_async_method`].
///
/// This trait is NOT designed to implemented on anything except bare handler functions, as below.
pub trait Wrap<const ARITY: usize, Ctx, Args, R> {
    type Future: Future<Output = Result<serde_json::Value, JsonRpcError>> + Send;
    fn wrap(
        self,
        param_names: [&'static str; ARITY],
        calling_convention: ParamStructure,
    ) -> impl Clone
           + Send
           + Sync
           + 'static
           + Fn(jsonrpsee::types::Params<'static>, Arc<Ctx>) -> Self::Future;
}

/// Return type-specific information for a function signature so that a [`ContentDescriptor`] can be created.
pub trait GenerateSchemas {
    fn generate_schemas(gen: &mut SchemaGenerator) -> Vec<(Schema, bool)>;
}

/// Implement [`Wrap`] and [`GenerateSchemas`] for various function arities.
///
/// See documentation on [`Wrap`] for more.
macro_rules! do_impls {
    ($arity:literal $(, $arg:ident)* $(,)?) => {
        const _: () = {
            let _assert: [&str; $arity] = [$(stringify!($arg)),*];
        };

        impl<F, $($arg,)* Ctx, Fut, R> Wrap<$arity, Ctx, ($($arg,)*), R> for F
        where
            F: Clone + Send + Sync + 'static + Fn(Arc<Ctx>, $($arg),*) -> Fut,
            //                                    ^^^ Receives an Arc rather than a reference lest
            //                                        we get the dreaded "one type is more general than the other"
            Ctx: Send + Sync + 'static,
            $(
                $arg: for<'de> Deserialize<'de> + Clone,
            )*
            Fut: Future<Output = Result<R, JsonRpcError>> + Send + Sync + 'static,
            R: Serialize,
        {
            type Future = Either<
                Ready<Result<Value, JsonRpcError>>, // early exit when parsing
                AndThenDeserializeResponse<Fut>,    // deferred handler, then deserializer
            >;

            fn wrap(
                self,
                param_names: [&'static str; $arity],
                calling_convention: ParamStructure,
            ) -> impl Clone
                   + Send
                   + Sync
                   + 'static
                   + Fn(jsonrpsee::types::Params<'static>, Arc<Ctx>) -> Self::Future {
                move |params, ctx| {
                    let params = match params.as_str().map(serde_json::from_str).transpose() {
                        Ok(it) => it,
                        Err(e) => {
                            return Either::Left(ready(Err(
                                JsonRpcError::invalid_params(e, None),
                            )));
                        }
                    };
                    #[allow(unused_variables, unused_mut)] // for the zero-argument case
                    let mut parser = match Parser::new(params, &param_names, calling_convention) {
                        Ok(it) => it,
                        Err(e) => return Either::Left(ready(Err(e))),
                    };

                    Either::Right(AndThenDeserializeResponse {
                        inner: self(
                            ctx,
                            $(  // parse out each argument
                                match parser.parse::<$arg>() {
                                    Ok(it) => it,
                                    Err(e) => return Either::Left(ready(Err(e))),
                                },
                            )*
                        ),
                    })
                }
            }
        }

        #[automatically_derived]
        impl<$($arg,)*> GenerateSchemas for ($($arg,)*)
        where
            $($arg: JsonSchema + for<'de> Deserialize<'de>,)*
        {
            fn generate_schemas(gen: &mut SchemaGenerator) -> Vec<(Schema, bool)> {
                vec![
                    $(($arg::json_schema(gen), $arg::optional())),*
                ]
            }
        }

    };
}

do_impls!(0);
do_impls!(1, T0);
do_impls!(2, T0, T1);
do_impls!(3, T0, T1, T2);
do_impls!(4, T0, T1, T2, T3);
do_impls!(5, T0, T1, T2, T3, T4);
do_impls!(6, T0, T1, T2, T3, T4, T5);
do_impls!(7, T0, T1, T2, T3, T4, T5, T6);
do_impls!(8, T0, T1, T2, T3, T4, T5, T6, T7);
do_impls!(9, T0, T1, T2, T3, T4, T5, T6, T7, T8);
do_impls!(10, T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);

pin_project! {
    pub struct AndThenDeserializeResponse<F> {
        #[pin]
        inner: F
    }
}

impl<R, F> Future for AndThenDeserializeResponse<F>
where
    F: Future<Output = Result<R, JsonRpcError>>,
    R: Serialize,
{
    type Output = Result<Value, JsonRpcError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(
            serde_json::to_value(ready!(self.project().inner.poll(cx))?).map_err(|e| {
                JsonRpcError::internal_error(
                    "error deserializing return value for handler",
                    json!({
                        "type": std::any::type_name::<R>(),
                        "error": e.to_string()
                    }),
                )
            }),
        )
    }
}