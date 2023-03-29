// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Implementation of the `derive_impl` attribute macro.

use macro_magic::core::pretty_print;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, ToTokens};
use std::collections::HashSet;
use syn::{
	braced, bracketed,
	parse::{Parse, ParseStream},
	parse2, parse_quote,
	punctuated::Punctuated,
	token::{Brace, Bracket},
	Ident, ImplItem, ItemImpl, Path, Result, Token, TypePath,
};

mod keywords {
	use syn::custom_keyword;

	custom_keyword!(derive_impl);
	custom_keyword!(partial_impl_block);
	custom_keyword!(implementing_type);
	custom_keyword!(type_items);
	custom_keyword!(fn_items);
	custom_keyword!(const_items);
}

pub struct DeriveImplDef {
	/// The partial impl block that the user provides. This should be interpreted as "override".
	partial_impl_block: ItemImpl,
	/// The full path to the type that can be used to receive defaults form.
	implementing_type: TypePath,
	/// All of the associated type items that we must eventually implement.
	type_items: Punctuated<Ident, Token![,]>,
	/// All of the function items that we must eventually implement.
	fn_items: Punctuated<Ident, Token![,]>,
	/// All of the constant items that we must eventually implement.
	const_items: Punctuated<Ident, Token![,]>,
}

impl Parse for DeriveImplDef {
	fn parse(input: ParseStream) -> Result<Self> {
		// NOTE: unfortunately, the order the keywords here must match what the pallet macro
		// expands. We can probably used a shared set of keywords later.
		let mut partial_impl_block;
		let _ = input.parse::<keywords::partial_impl_block>()?;
		let _ = input.parse::<Token![=]>()?;
		let _replace_with_bracket: Bracket = bracketed!(partial_impl_block in input);
		let _replace_with_brace: Brace = braced!(partial_impl_block in partial_impl_block);
		let partial_impl_block = partial_impl_block.parse()?;

		let mut implementing_type;
		let _ = input.parse::<keywords::implementing_type>()?;
		let _ = input.parse::<Token![=]>()?;
		let _replace_with_bracket: Bracket = bracketed!(implementing_type in input);
		let _replace_with_brace: Brace = braced!(implementing_type in implementing_type);
		let implementing_type = implementing_type.parse()?;

		let mut type_items;
		let _ = input.parse::<keywords::type_items>()?;
		let _ = input.parse::<Token![=]>()?;
		let _replace_with_bracket: Bracket = bracketed!(type_items in input);
		let _replace_with_brace: Brace = braced!(type_items in type_items);
		let type_items = Punctuated::<Ident, Token![,]>::parse_terminated(&type_items)?;

		let mut fn_items;
		let _ = input.parse::<keywords::fn_items>()?;
		let _ = input.parse::<Token![=]>()?;
		let _replace_with_bracket: Bracket = bracketed!(fn_items in input);
		let _replace_with_brace: Brace = braced!(fn_items in fn_items);
		let fn_items = Punctuated::<Ident, Token![,]>::parse_terminated(&fn_items)?;

		let mut const_items;
		let _ = input.parse::<keywords::const_items>()?;
		let _ = input.parse::<Token![=]>()?;
		let _replace_with_bracket: Bracket = bracketed!(const_items in input);
		let _replace_with_brace: Brace = braced!(const_items in const_items);
		let const_items = Punctuated::<Ident, Token![,]>::parse_terminated(&const_items)?;

		Ok(Self { partial_impl_block, type_items, fn_items, const_items, implementing_type })
	}
}

pub(crate) fn derive_impl_inner(input: TokenStream2) -> Result<TokenStream2> {
	println!("input: {}", input);
	let DeriveImplDef { partial_impl_block, implementing_type, type_items, .. } = parse2(input)?;

	let type_item_name = |i: &ImplItem| {
		if let ImplItem::Type(t) = i {
			t.ident.clone()
		} else {
			panic!("only support type items for now")
		}
	};

	// might be able to mutate `partial_impl_block` along the way, but easier like this for now.
	let mut final_impl_block = partial_impl_block.clone();
	let source_crate_path = implementing_type.path.segments.first().unwrap().ident.clone();

	// TODO: ensure type ident specified in `partial_impl_block` is beyond union(type_items,
	// const_items, fn_items).
	assert!(
		partial_impl_block
			.items
			.iter()
			.all(|i| { type_items.iter().find(|tt| tt == &&type_item_name(i)).is_some() }),
		"some item in the partial_impl_block is unexpected"
	);

	// for each item that is in `type_items` but not present in `partial_impl_block`, fill it in.
	type_items.iter().for_each(|ident| {
		if partial_impl_block.items.iter().any(|i| &type_item_name(i) == ident) {
			// this is already present in the partial impl block -- noop
		} else {
			// add it
			let tokens = quote::quote!(type #ident = <#implementing_type as #source_crate_path::pallet::DefaultConfig>::#ident;);
			let parsed: ImplItem = parse2(tokens).expect("it is a valid type item");
			debug_assert!(matches!(parsed, ImplItem::Type(_)));

			final_impl_block.items.push(parsed)
		}
	});

	Ok(quote::quote!(#final_impl_block))
}

fn impl_item_ident(impl_item: &ImplItem) -> Option<Ident> {
	match impl_item {
		ImplItem::Const(item) => Some(item.ident.clone()),
		ImplItem::Method(item) => Some(item.sig.ident.clone()),
		ImplItem::Type(item) => Some(item.ident.clone()),
		_ => None,
	}
}

fn combine_impls(local_impl: ItemImpl, foreign_impl: ItemImpl, foreign_path: Path) -> ItemImpl {
	let existing_local_keys: HashSet<Ident> = local_impl
		.items
		.iter()
		.filter_map(|impl_item| impl_item_ident(impl_item))
		.collect();
	let existing_unsupported_items: HashSet<ImplItem> = local_impl
		.items
		.iter()
		.filter(|impl_item| impl_item_ident(impl_item).is_none())
		.cloned()
		.collect();
	let source_crate_path = foreign_path.segments.first().unwrap().ident.clone();
	let mut final_impl = local_impl;
	final_impl.items.extend(
		foreign_impl
			.items
			.into_iter()
			.filter_map(|item| {
				if let Some(ident) = impl_item_ident(&item) {
					if existing_local_keys.contains(&ident) {
						// do not copy colliding items that have an ident
						None
					} else {
						if matches!(item, ImplItem::Type(_)) {
							// modify and insert uncolliding type items
							let modified_item: ImplItem = parse_quote! {
								type #ident = <#foreign_path as #source_crate_path::pallet::DefaultConfig>::#ident;
							};
							Some(modified_item)
						} else {
							// copy uncolliding non-type items that have an ident
							Some(item)
						}
					}
				} else {
					if existing_unsupported_items.contains(&item) {
						// do not copy colliding items that lack an ident
						None
					} else {
						// copy uncolliding items without an ident verbaitm
						Some(item)
					}
				}
			})
			.collect::<Vec<ImplItem>>(),
	);
	final_impl
}

pub fn derive_impl(
	foreign_path: TokenStream2,
	foreign_tokens: TokenStream2,
	local_tokens: TokenStream2,
) -> Result<TokenStream2> {
	println!("foreign_path: {}\n", foreign_path.to_string());
	println!("foreign_impl:");
	pretty_print(&foreign_tokens);
	println!("\nlocal_impl:");
	pretty_print(&local_tokens);

	let local_impl = parse2::<ItemImpl>(local_tokens)?;
	let foreign_impl = parse2::<ItemImpl>(foreign_tokens)?;
	let foreign_path = parse2::<Path>(foreign_path)?;

	let combined_impl = combine_impls(local_impl, foreign_impl, foreign_path);

	println!("combined_impl:");
	pretty_print(&combined_impl.to_token_stream());

	Ok(quote!(#combined_impl))
	// attr: frame_system::preludes::testing::Impl
	// tokens:
	// impl frame_system::Config for Test {
	//	// These are all defined by system as mandatory.
	// 	type BaseCallFilter = frame_support::traits::Everything;
	// 	type RuntimeEvent = RuntimeEvent;
	// 	type RuntimeCall = RuntimeCall;
	// 	type RuntimeOrigin = RuntimeOrigin;
	// 	type OnSetCode = frame_system::DefaultSetCode<Self>;
	// 	type PalletInfo = PalletInfo;
	// 	type Header = Header;
	// 	// We decide to override this one.
	// 	type AccountData = pallet_balances::AccountData<u64>;
	// }
	// let implementing_type: TypePath = parse2(attrs.clone())?;
	// // ideas for sam:
	// // let other_path_tokens = magic_macro!(path_to_other_path_token);
	// // let foreign_trait_tokens = import_tokens_indirect!(frame_system::testing::DefaultConfig);
	// // println!("{}", foreign_trait_tokens.to_string());

	// let frame_support = generate_crate_access_2018("frame-support")?;
	// // TODO: may not be accurate.
	// let source_crate_path = implementing_type.path.segments.first().unwrap().ident.clone();
	// // source_crate_path = frame_system

	// //let tokens = import_tokens_indirect!(::pallet_example_basic::pallet::Config);

	// Ok(quote::quote!(
	// 	#frame_support::tt_call! {
	// 		macro = [{ #source_crate_path::tt_config_items }] // frame_system::tt_config_items
	// 		frame_support = [{ #frame_support }] // ::frame_support
	// 		~~> #frame_support::derive_impl_inner! {
	// 			partial_impl_block = [{ #input }]
	// 			implementing_type = [{ #attrs }]
	// 		}
	// 	}
	// ))
}