use convert_case::{Case, Casing};
use core::panic;
use proc_macro2::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use std::{
    collections::{HashMap, HashSet},
    fs,
};
use swc_common::{util::move_map::MoveMap, TypeEq};
use swc_ecma_ast::{
    BindingIdent, CallExpr, Callee, ClassExpr, ClassMethod, Expr, ExprOrSpread, Lit, MemberExpr,
    NewExpr, Stmt, TsExprWithTypeArgs, TsInterfaceDecl, TsKeywordTypeKind, TsType, TsTypeRef,
};
use swc_ecma_parser::token::Token;

use crate::{
    errors::PoseidonError,
    ts_types::{rs_type_from_str, struct_rs_type_from_str, STANDARD_ACCOUNT_TYPES, STANDARD_ARRAY_TYPES, STANDARD_TYPES},
};
use anyhow::{anyhow, Error, Ok, Result};

#[derive(Debug, Clone)]
pub struct Ta {
    mint: String,
    authority: String,
    is_ata: bool,
}
#[derive(Clone, Debug)]

pub struct InstructionAccount {
    pub name: String,
    pub of_type: TokenStream,
    pub type_str: String,
    pub optional: bool,
    pub is_mut: bool,
    pub is_init: bool,
    pub is_initifneeded: bool,
    pub is_close: bool,
    pub is_mint: bool,
    pub ta: Option<Ta>,
    pub has_one: Vec<String>,
    pub close: Option<String>,
    pub seeds: Option<Vec<TokenStream>>,
    pub bump: Option<TokenStream>,
    pub payer: Option<String>,
    pub space: Option<u16>,
}

impl InstructionAccount {
    pub fn new(name: String, of_type: TokenStream, type_str: String, optional: bool) -> Self {
        Self {
            name: name.to_case(Case::Snake),
            of_type,
            type_str,
            optional,
            is_mut: false,
            is_close: false,
            is_init: false,
            is_initifneeded: false,
            is_mint: false,
            ta: None,
            has_one: vec![],
            close: None,
            seeds: None,
            bump: None,
            payer: None,
            space: None,
        }
    }

    pub fn to_tokens(&self, ix_a: &Vec<Ident>, realloc: &bool) -> TokenStream {
        let name = Ident::new(&self.name, proc_macro2::Span::call_site());
        let of_type = &self.of_type;
        let constraints: TokenStream;
        let payer = match &self.payer {
            Some(s) => {
                let payer = Ident::new(s, proc_macro2::Span::call_site());
                quote!(
                    payer = #payer
                )
            }
            None => quote!(),
        };

        let ata = match &self.ta {
            Some(a) => {
                let mint = Ident::new(&a.mint, proc_macro2::Span::call_site());
                let authority = Ident::new(&a.authority, proc_macro2::Span::call_site());
                if a.is_ata {
                    quote! {
                        associated_token::mint = #mint,
                        associated_token::authority = #authority,
                    }
                } else {
                    quote! {
                        token::mint = #mint,
                        token::authority = #authority,
                    }
                }
            }
            None => quote!(),
        };
        let close = match &self.close {
            Some(c) => {
                let close_acc = Ident::new(c, proc_macro2::Span::call_site());

                quote! {
                    close = #close_acc,
                }
            }
            None => quote!(),
        };

        let seeds = match &self.seeds {
            Some(s) => {
                quote! {
                    seeds = [#(#s),*],
                }
            }
            None => quote! {},
        };

        let bump = match &self.bump {
            Some(b) => {
                quote! {
                    #b,
                }
            }
            None => quote! {},
        };


        // input1.len() + input2.len() + ...
        let mut input_lengths: Vec<TokenStream> = vec![];
        for s in ix_a {
            input_lengths.push(quote!( + #s.len() ));
        }

        // space value + input1.len() + input2.len() + ...
        let space = match self.space {
            Some(s) => {
                let s_literal = Literal::u16_unsuffixed(s);
                quote ! {
                    space = #s_literal #(#input_lengths)*,
                }
            }
            None => {
                quote! {}
            }
        };

        
        println!("input_lengths {:?}", input_lengths);
        println!("space {:?}", space);
        println!("realloc {:?}", realloc);


        let realloc = match realloc {
            true => {
                quote! {
                    mut,
                    realloc = 8 #(#input_lengths)*,
                    realloc::payer = #payer,
                    realloc::zero = true,
                }
            }
            false => quote! {},
        };

        let init = match self.is_init {
            true => quote! {init, #payer, #space},
            false => quote! {},
        };

        let mutable = match self.is_mut && !(self.is_init || self.is_initifneeded) {
            true => quote! {mut,},
            false => quote! {},
        };
        let mut has: TokenStream = quote! {};
        if !self.has_one.is_empty() {
            let mut has_vec: Vec<TokenStream> = vec![];
            for h in &self.has_one {
                let h_ident = Ident::new(h, proc_macro2::Span::call_site());
                has_vec.push(quote! {
                    has_one = #h_ident
                })
            }
            has = quote! { #(#has_vec),*,};
        }
        let init_if_needed = match self.is_initifneeded {
            true => quote! {init_if_needed, #payer,},
            false => quote! {},
        };

        if self.is_mint {
            constraints = quote! {}
        } else {
            constraints = quote! {
                #[account(
                    #init
                    #init_if_needed
                    #mutable
                    #seeds
                    #ata
                    #has
                    #bump
                    #close
                    #realloc

                )]
            }
        }
        let check = if self.type_str == "UncheckedAccount" {
            quote! {
                /// CHECK: This acc is safe
            }
        } else {
            quote! {}
        };
        quote!(
            #constraints
            #check
            pub #name: #of_type,
        )
    }
}

#[derive(Clone, Debug)]

pub struct InstructionArgument {
    pub name: String,
    pub of_type: TokenStream,
    pub optional: bool,
}

#[derive(Clone, Debug)]
pub struct InstructionAttributes {
    pub token_streams: Vec<TokenStream>,
    pub string_idents: Vec<Ident>
}

#[derive(Clone, Debug)]
pub struct ProgramInstruction {
    pub name: String,
    pub accounts: Vec<InstructionAccount>,
    pub args: Vec<InstructionArgument>,
    pub body: Vec<TokenStream>,
    pub signer: Option<String>,
    pub uses_system_program: bool,
    pub uses_token_program: bool,
    pub uses_associated_token_program: bool,
    pub instruction_attributes: InstructionAttributes,
    pub realloc: bool,
}

impl ProgramInstruction {
    pub fn new(name: String) -> Self {
        Self {
            name,
            accounts: vec![],
            args: vec![],
            body: vec![],
            signer: None,
            uses_system_program: false,
            uses_token_program: false,
            uses_associated_token_program: false,
            instruction_attributes: InstructionAttributes {
                token_streams: vec![],
                string_idents: vec![],
            },
            realloc: false,
        }
    }
    pub fn get_amount_from_ts_arg(amount_expr: &Expr) -> Result<TokenStream> {
        let amount: TokenStream;
        match amount_expr {
            Expr::Member(m) => {
                let amount_obj = m
                    .obj
                    .as_ident()
                    .ok_or(PoseidonError::IdentNotFound)?
                    .sym
                    .as_ref();
                let amount_prop = m
                    .prop
                    .as_ident()
                    .ok_or(PoseidonError::IdentNotFound)?
                    .sym
                    .as_ref();
                let amount_obj_ident = Ident::new(
                    &amount_obj.to_case(Case::Snake),
                    proc_macro2::Span::call_site(),
                );
                let amount_prop_ident = Ident::new(
                    &amount_prop.to_case(Case::Snake),
                    proc_macro2::Span::call_site(),
                );
                amount = quote! {
                    ctx.accounts.#amount_obj_ident.#amount_prop_ident
                };
            }
            Expr::Ident(i) => {
                let amount_str = i.sym.as_ref();
                let amount_ident = Ident::new(
                    &amount_str.to_case(Case::Snake),
                    proc_macro2::Span::call_site(),
                );
                amount = quote! {
                    #amount_ident
                };
            }
            _ => {
                panic!("amount not  provided in proper format")
            }
        }
        Ok(amount)
    }
    pub fn get_instruction_attributes(&mut self, elems: &Vec<Option<ExprOrSpread>>) -> Result<InstructionAttributes> {
        let mut ix_attribute_token: Vec<TokenStream> = vec![];
        let mut ix_strings_list: Vec<Ident> = vec![];

        for elem in elems.into_iter().flatten() {
            let arg_name = elem.expr
                .as_ident()
                .ok_or(PoseidonError::IdentNotFound)?
                .sym.to_string()
                .to_case(Case::Snake);
            let arg_ident = Ident::new(&arg_name, Span::call_site());

            for arg in self.args.iter() {
                if arg.name == arg_name {
                    let type_ident = &arg.of_type;
                    
                    ix_attribute_token.push(quote! {
                        #arg_ident : #type_ident
                    });

                    let is_string_attribute = type_ident
                        .to_string()
                        .contains("tring");

                    if is_string_attribute {
                        ix_strings_list.push(Ident::new(arg_name.clone().as_str(), Span::call_site()));
                    }
                }
            }
        }

        Ok(InstructionAttributes {
            token_streams: ix_attribute_token,
            string_idents: ix_strings_list,
        })
    }
    pub fn get_seeds(&mut self, seeds: &Vec<Option<ExprOrSpread>>) -> Result<Vec<TokenStream>> {
        let mut seeds_token: Vec<TokenStream> = vec![];
        
        for elem in seeds.into_iter().flatten() {
            match *(elem.expr.clone()) {
                Expr::Lit(Lit::Str(seedstr)) => {
                    let lit_vec = Literal::byte_string(seedstr.value.as_bytes());
                    seeds_token.push(quote! {
                    #lit_vec
                    });
                }
                Expr::Ident(ident_str) => {
                    let seed_ident =
                        Ident::new(ident_str.sym.as_ref(), proc_macro2::Span::call_site());
                    seeds_token.push(quote! {
                        #seed_ident
                    });
                }
                Expr::Member(m) => {
                    let seed_prop = Ident::new(
                        m.prop
                            .as_ident()
                            .ok_or(PoseidonError::IdentNotFound)?
                            .sym
                            .as_ref(),
                        Span::call_site(),
                    );
                    let seed_obj = Ident::new(
                        m.obj
                            .as_ident()
                            .ok_or(PoseidonError::IdentNotFound)?
                            .sym
                            .as_ref(),
                        Span::call_site(),
                    );
                    seeds_token.push(quote! {
                        #seed_obj.#seed_prop().as_ref()
                    })
                }
                Expr::Call(c) => {
                    let seed_members = c
                        .callee
                        .as_expr()
                        .ok_or(PoseidonError::ExprNotFound)?
                        .as_member()
                        .ok_or(PoseidonError::MemberNotFound)?;
                    if seed_members.obj.is_ident() {
                        let seed_obj = seed_members
                            .obj
                            .as_ident()
                            .ok_or(PoseidonError::IdentNotFound)?
                            .sym
                            .as_ref();
                        let seed_obj_ident = Ident::new(seed_obj, Span::call_site());
                        if seed_members
                            .prop
                            .as_ident()
                            .ok_or(PoseidonError::IdentNotFound)?
                            .sym
                            .as_ref()
                            == "toBytes"
                        {
                            seeds_token.push(quote! {
                                #seed_obj_ident.to_le_bytes().as_ref()
                            });
                        }

                    } else if seed_members.obj.is_member() {
                        if seed_members
                            .prop
                            .as_ident()
                            .ok_or(PoseidonError::IdentNotFound)?
                            .sym
                            .as_ref()
                            == "toBytes"
                        {
                            let seed_obj_ident = Ident::new(
                                seed_members
                                    .obj
                                    .clone()
                                    .expect_member()
                                    .obj
                                    .expect_ident()
                                    .sym
                                    .as_ref(),
                                Span::call_site(),
                            );
                            let seed_prop_ident = Ident::new(
                                seed_members
                                    .obj
                                    .as_member()
                                    .ok_or(PoseidonError::MemberNotFound)?
                                    .prop
                                    .as_ident()
                                    .ok_or(PoseidonError::IdentNotFound)?
                                    .sym
                                    .as_ref(),
                                Span::call_site(),
                            );
                            seeds_token.push(quote! {
                                #seed_obj_ident.#seed_prop_ident.to_le_bytes().as_ref()
                            })
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(seeds_token)
    }

    pub fn from_class_method(
        program_mod: &mut ProgramModule,
        c: &ClassMethod,
        custom_accounts: &HashMap<String, ProgramAccount>,
    ) -> Result<Self> {
        // Get name
        let name = c
            .key
            .as_ident()
            .ok_or(PoseidonError::IdentNotFound)?
            .sym
            .to_string();
        // println!("{}",name);
        let mut ix: ProgramInstruction = ProgramInstruction::new(name);
        // Get accounts and args
        let mut ix_accounts: HashMap<String, InstructionAccount> = HashMap::new();
        let mut ix_arguments: Vec<InstructionArgument> = vec![];
        let mut ix_body: Vec<TokenStream> = vec![];
        c.function.params.iter().for_each(|p| {
            let BindingIdent { id, type_ann } = p.pat.clone().expect_ident();
            let name = id.sym.to_string();
            let snaked_name = id.sym.to_string().to_case(Case::Snake);
            let binding = type_ann.expect("Invalid type annotation");
            let (of_type, optional) = extract_of_type(binding)
                .unwrap_or_else(|_| panic!("Keyword type is not supported"));

            // TODO: Make this an actual Enum set handle it correctly
            if STANDARD_TYPES.contains(&of_type.as_str()) | STANDARD_ARRAY_TYPES.contains(&of_type.as_str()) {
                ix_arguments.push(InstructionArgument {
                    name: snaked_name,
                    of_type: rs_type_from_str(&of_type)
                        .unwrap_or_else(|_| panic!("Invalid type: {}", of_type)),
                    optional,
                })
            } else if STANDARD_ACCOUNT_TYPES.contains(&of_type.as_str()) {
                if of_type == "Signer" {
                    ix.signer = Some(name.clone());
                    ix_accounts.insert(
                        name.clone(),
                        InstructionAccount::new(
                            snaked_name.clone(),
                            quote! { Signer<'info> },
                            of_type,
                            optional,
                        ),
                    );
                    let cur_ix_acc = ix_accounts.get_mut(&name.clone()).unwrap();
                    cur_ix_acc.is_mut = true;
                } else if of_type == "UncheckedAccount" {
                    ix_accounts.insert(
                        name.clone(),
                        InstructionAccount::new(
                            snaked_name.clone(),
                            quote! { UncheckedAccount<'info> },
                            of_type,
                            optional,
                        ),
                    );
                } else if of_type == "SystemAccount" {
                    ix_accounts.insert(
                        name.clone(),
                        InstructionAccount::new(
                            snaked_name.clone(),
                            quote! { SystemAccount<'info> },
                            of_type,
                            optional,
                        ),
                    );
                    ix.uses_system_program = true;

                    let cur_ix_acc = ix_accounts.get_mut(&name.clone()).unwrap();
                    cur_ix_acc.is_mut = true;
                } else if of_type == "AssociatedTokenAccount" {
                    ix_accounts.insert(
                        name.clone(),
                        InstructionAccount::new(
                            snaked_name.clone(),
                            quote! { Account<'info, TokenAccount> },
                            of_type,
                            optional,
                        ),
                    );
                    ix.uses_associated_token_program = true;
                    ix.uses_token_program = true;

                    program_mod.add_import("anchor_spl", "associated_token", "AssociatedToken");
                } else if of_type == "Mint" {
                    ix_accounts.insert(
                        name.clone(),
                        InstructionAccount::new(
                            snaked_name.clone(),
                            quote! { Account<'info, Mint> },
                            of_type,
                            optional,
                        ),
                    );
                    program_mod.add_import("anchor_spl", "token", "Mint");
                } else if of_type == "TokenAccount" {
                    ix_accounts.insert(
                        name.clone(),
                        InstructionAccount::new(
                            snaked_name.clone(),
                            quote! { Account<'info, TokenAccount> },
                            of_type,
                            optional,
                        ),
                    );
                    ix.uses_token_program = true;
                    program_mod.add_import("anchor_spl", "token", "TokenAccount");
                    program_mod.add_import("anchor_spl", "token", "Token");
                }
            } else if custom_accounts.contains_key(&of_type) {
                let ty = Ident::new(&of_type, proc_macro2::Span::call_site());
                ix_accounts.insert(
                    name.clone(),
                    InstructionAccount::new(
                        snaked_name.clone(),
                        quote! { Account<'info, #ty> },
                        of_type.clone(),
                        optional,
                    ),
                );
                ix.uses_system_program = true;
                let cur_ix_acc = ix_accounts.get_mut(&name.clone()).unwrap();
                cur_ix_acc.space = Some(
                    custom_accounts
                        .get(&of_type)
                        .expect("space for custom acc not found")
                        .space,
                );
            } else {
                panic!("Invalid variable or account type: {}", of_type);
            }
        });
        ix.args = ix_arguments;

        let _ = c.clone()
            .function
            .body
            .ok_or(anyhow!("block statement none"))
            ?.stmts
            .iter()
            .map(|s| {
                // println!("start : {:#?}", s);
                match s.clone() {
                    Stmt::Expr(e) => {
                        let s = e.expr;
                        match *s {
                            Expr::Call(c) => {
                                let parent_call = c.callee.as_expr().ok_or(PoseidonError::ExprNotFound)?.as_member().ok_or(PoseidonError::MemberNotFound)?;
                                let members: &MemberExpr;
                                let mut obj = "";
                                let mut prop = "";
                                let mut derive_args: &Vec<ExprOrSpread> = &vec![];
                                if parent_call.obj.is_call() {
                                    members = parent_call
                                        .obj
                                        .as_call()
                                        .ok_or(PoseidonError::CallNotFound)
                                        ?.callee
                                        .as_expr()
                                        .ok_or(PoseidonError::ExprNotFound)
                                        ?.as_member()
                                        .ok_or(PoseidonError::MemberNotFound)?;
                                    if members.obj.is_ident(){
                                        obj = members.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        prop = members.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        if prop == "derive" {
                                            derive_args = &parent_call.obj.as_call().ok_or(PoseidonError::CallNotFound)?.args;
                                        }
                                    } else if members.obj.is_call() {
                                        let sub_members = members.obj.as_call().ok_or(PoseidonError::CallNotFound)?.callee.as_expr().ok_or(PoseidonError::ExprNotFound)?.as_member().ok_or(PoseidonError::MemberNotFound)?;
                                        obj = sub_members.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        prop = sub_members.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        if prop == "derive" {
                                            derive_args = &members.obj.as_call().ok_or(PoseidonError::CallNotFound)?.args;
                                        }
                                    }
                                } else if parent_call.obj.is_ident() {
                                    obj = parent_call.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                    prop = parent_call.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                    if prop.contains("derive") {
                                        derive_args = &c.args;
                                    }
                                }

                                if let Some(cur_ix_acc) = ix_accounts.get_mut(obj) {
                                    if prop.contains("derive") {
                                        let chaincall1prop = c
                                            .callee
                                            .as_expr()
                                            .ok_or(PoseidonError::ExprNotFound)
                                            ?.as_member()
                                            .ok_or(PoseidonError::MemberNotFound)
                                            ?.prop
                                            .as_ident()
                                            .ok_or(PoseidonError::IdentNotFound)
                                            ?.sym
                                            .as_ref();
                                        let mut chaincall2prop = "";
                                        if c.clone().callee.expect_expr().expect_member().obj.is_call(){
                                            chaincall2prop = c
                                                                .callee
                                                                .as_expr()
                                                                .ok_or(PoseidonError::ExprNotFound)
                                                                ?.as_member()
                                                                .ok_or(PoseidonError::MemberNotFound)
                                                                ?.obj
                                                                .as_call()
                                                                .ok_or(PoseidonError::CallNotFound)
                                                                ?.callee
                                                                .as_expr()
                                                                .ok_or(PoseidonError::ExprNotFound)
                                                                ?.as_member()
                                                                .ok_or(PoseidonError::MemberNotFound)
                                                                ?.prop
                                                                .as_ident()
                                                                .ok_or(PoseidonError::IdentNotFound)
                                                                ?.sym
                                                                .as_ref();
                                        }
                                        if cur_ix_acc.type_str == "AssociatedTokenAccount" {
                                            let mint = derive_args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let ata_auth = derive_args[1].expr.as_member().ok_or(PoseidonError::MemberNotFound)?.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            cur_ix_acc.ta = Some(
                                                Ta {
                                                    mint: mint.to_case(Case::Snake),
                                                    authority: ata_auth.to_case(Case::Snake),
                                                    is_ata: true,
                                                }
                                            );
                                            cur_ix_acc.is_mut = true;
                                        } else if cur_ix_acc.type_str == "TokenAccount" {
                                            let mint = derive_args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let ta_auth = derive_args[2].expr.as_member().ok_or(PoseidonError::MemberNotFound)?.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            cur_ix_acc.ta = Some(
                                                Ta {
                                                    mint: mint.to_case(Case::Snake),
                                                    authority: ta_auth.to_case(Case::Snake),
                                                    is_ata: false,
                                                }
                                            );
                                            cur_ix_acc.is_mut = true;
                                        }
                                        if cur_ix_acc.type_str != "AssociatedTokenAccount"{
                                            let seeds = &derive_args[0].expr.as_array().ok_or(anyhow!("expected a array"))?.elems;
                                            let seeds_token = ix.get_seeds(seeds)?;
                                            cur_ix_acc.bump = Some(quote!{
                                                bump
                                            });
                                            if !seeds_token.is_empty() {
                                                cur_ix_acc.seeds = Some(seeds_token);
                                            }
                                        }
                                        if prop == "deriveWithBump" {
                                            let bump_members = c.args.last().ok_or(anyhow!("no last element in vector"))?.expr.as_member().ok_or(PoseidonError::MemberNotFound)?;
                                            let bump_prop  = Ident::new(
                                                &bump_members.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref().to_case(Case::Snake),
                                                Span::call_site(),
                                            );
                                            let bump_obj = Ident::new(
                                                &bump_members.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref().to_case(Case::Snake),
                                                Span::call_site(),
                                            );
                                            cur_ix_acc.bump = Some(quote!{
                                                bump = #bump_obj.#bump_prop
                                            })
                                        }

                                        println!("chaincall1prop {:?}", chaincall1prop);
                                        println!("chaincall2prop {:?}", chaincall2prop);


                                        if chaincall2prop == "instruction" {
                                            let elems = &c.callee.as_expr()
                                                .ok_or(PoseidonError::ExprNotFound)?
                                                .as_member()
                                                .ok_or(PoseidonError::MemberNotFound)?
                                                .obj.as_call()
                                                .ok_or(PoseidonError::CallNotFound)?
                                                .args[0]
                                                .expr.as_array()
                                                .ok_or(anyhow!("expected a array"))?
                                                .elems;

                                            // Retrieve the second argument as a boolean
                                            let second_arg = c.callee.as_expr().ok_or(PoseidonError::ExprNotFound)?
                                                .as_member().ok_or(PoseidonError::MemberNotFound)?
                                                .obj.as_call().ok_or(PoseidonError::CallNotFound)?
                                                .args.get(1); // Get the second argument

                                            ix.realloc = match second_arg {
                                                Some(arg) => match &*arg.expr {
                                                    Expr::Lit(Lit::Bool(b)) => b.value,
                                                    _ => false, // Default to false if not a boolean literal
                                                },
                                                None => false, // Default to false if the second argument is not present
                                            };
                                            
                                            ix.instruction_attributes = ix.get_instruction_attributes(elems).expect("instruction attributes not found");
                                        }

                                        if chaincall1prop == "init" {
                                            ix.uses_system_program = true;
                                            cur_ix_acc.is_init = true;
                                            if let Some(payer) = &ix.signer {
                                                cur_ix_acc.payer = Some(payer.clone());
                                            }
                                        }
                                        else if chaincall1prop == "initIfNeeded" {
                                            ix.uses_system_program = true;
                                            cur_ix_acc.is_initifneeded = true;
                                            if let Some(payer) = &ix.signer {
                                                cur_ix_acc.payer = Some(payer.clone());
                                            }
                                        }
                                        if chaincall1prop == "close" {
                                            cur_ix_acc.close = Some(c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref().to_case(Case::Snake));
                                            cur_ix_acc.is_mut = true;
                                        }
                                        if chaincall2prop == "has" {
                                            let elems = &c.callee.as_expr().ok_or(PoseidonError::ExprNotFound)?.as_member().ok_or(PoseidonError::MemberNotFound)?.obj.as_call().ok_or(PoseidonError::CallNotFound)?.args[0].expr.as_array().ok_or(anyhow!("expected a array"))?.elems;
                                            let mut has_one:Vec<String> = vec![];
                                            for elem in elems.into_iter().flatten() {
                                                    has_one.push(elem.expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.to_string().to_case(Case::Snake));
                                            }
                                            cur_ix_acc.has_one = has_one;

                                        }
                                    }
                                }
                                if obj == "SystemProgram" {
                                    if prop == "transfer" {
                                        program_mod.add_import("anchor_lang", "system_program", "Transfer");
                                        program_mod.add_import("anchor_lang", "system_program", "transfer");
                                        let from_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        let to_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        let from_acc_ident = Ident::new(from_acc, proc_macro2::Span::call_site());
                                        let to_acc_ident = Ident::new(to_acc, proc_macro2::Span::call_site());
                                        let amount_expr = &c.args[2].expr;
                                        let amount = ProgramInstruction::get_amount_from_ts_arg(amount_expr)?;
                                        if let Some(cur_ix_acc) = ix_accounts.get(from_acc){
                                            if cur_ix_acc.seeds.is_some(){
                                                ix_body.push(quote!{
                                                    let transfer_accounts = Transfer {
                                                        from: ctx.accounts.#from_acc_ident.to_account_info(),
                                                        to: ctx.accounts.#to_acc_ident.to_account_info()
                                                    };

                                                    let seeds = &[
                                                        b"vault",
                                                        ctx.accounts.auth.to_account_info().key.as_ref(),
                                                        &[ctx.accounts.state.vault_bump],
                                                    ];

                                                    let pda_signer = &[&seeds[..]];

                                                    let cpi_ctx = CpiContext::new_with_signer(
                                                        ctx.accounts.system_program.to_account_info(),
                                                        transfer_accounts,
                                                        pda_signer
                                                    );
                                                    transfer(cpi_ctx, amount)?;
                                                });
                                            } else {
                                                ix_body.push(quote!{
                                                    let transfer_accounts = Transfer {
                                                        from: ctx.accounts.#from_acc_ident.to_account_info(),
                                                        to: ctx.accounts.#to_acc_ident.to_account_info()
                                                    };
                                                    let cpi_ctx = CpiContext::new(
                                                        ctx.accounts.system_program.to_account_info(),
                                                        transfer_accounts
                                                    );
                                                    transfer(cpi_ctx, #amount)?;
                                                });
                                            }
                                        }

                                    }
                                }

                                if obj == "TokenProgram" {
                                    match prop {
                                        "transfer" => {
                                        program_mod.add_import("anchor_spl", "token", "transfer");
                                        program_mod.add_import("anchor_spl", "token", "Transfer");
                                        let from_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        let to_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                        let from_acc_ident = Ident::new(&from_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                        let to_acc_ident = Ident::new(&to_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                        let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                        let amount_expr = &c.args[3].expr;
                                        let amount = ProgramInstruction::get_amount_from_ts_arg(amount_expr)?;
                                        if let Some(cur_ix_acc) = ix_accounts.get(from_acc){
                                            if cur_ix_acc.seeds.is_some() {
                                                ix_body.push(quote!{
                                                    let cpi_accounts = TransferSPL {
                                                        from: ctx.accounts.#from_acc_ident.to_account_info(),
                                                        to: ctx.accounts.#to_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    };
                                                    let signer_seeds = &[
                                                        &b"auth"[..],
                                                        &[ctx.accounts.escrow.auth_bump],
                                                    ];
                                                    let binding = [&signer_seeds[..]];
                                                    let cpi_ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, &binding);
                                                    transfer_spl(cpi_ctx, #amount)?;
                                                });
                                            } else {
                                                ix_body.push(quote!{
                                                    let cpi_accounts = TransferSPL {
                                                        from: ctx.accounts.#from_acc_ident.to_account_info(),
                                                        to: ctx.accounts.#to_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    };
                                                    let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
                                                    transfer_spl(cpi_ctx, #amount)?;
                                                })
                                            }
                                        }
                                        },
                                        "burn" => {
                                            let mint_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let from_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let mint_acc_ident = Ident::new(&mint_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let from_acc_ident = Ident::new(&from_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let amount_expr = &c.args[3].expr;
                                            let amount = ProgramInstruction::get_amount_from_ts_arg(amount_expr)?;

                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    Burn {
                                                        mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                        from: ctx.accounts.#from_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                burn(cpi_ctx, #amount)?;
                                            })
                                        },
                                        "mintTo" => {
                                            let mint_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let to_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let mint_acc_ident = Ident::new(&mint_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let to_acc_ident = Ident::new(&to_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let amount_expr = &c.args[3].expr;
                                            let amount = ProgramInstruction::get_amount_from_ts_arg(amount_expr)?;
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new_with_signer(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    MintTo {
                                                        mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                        to: ctx.accounts.#to_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                    signer,
                                                );
                                                mint_to(cpi_ctx, #amount)?;
                                            })
                                        },
                                        "approve" => {
                                            let to_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let delegate_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let to_acc_ident = Ident::new(&to_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let delegate_acc_ident = Ident::new(&delegate_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let amount_expr = &c.args[3].expr;
                                            let amount = ProgramInstruction::get_amount_from_ts_arg(amount_expr)?;
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    Approve {
                                                        to: ctx.accounts.#to_acc_ident.to_account_info(),
                                                        delegate: ctx.accounts.#delegate_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                approve(cpi_ctx, #amount)?;
                                            })
                                        },
                                        "approveChecked" => {
                                            let to_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let mint_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let delegate_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[3].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let to_acc_ident = Ident::new(&to_acc.to_case(Case::Snake),
                                            proc_macro2::Span::call_site());
                                            let mint_acc_ident = Ident::new(&mint_acc.to_case(Case::Snake),
                                            proc_macro2::Span::call_site());
                                            let delegate_acc_ident = Ident::new(&delegate_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let amount_expr = &c.args[4].expr;
                                            let amount = ProgramInstruction::get_amount_from_ts_arg(amount_expr)?;
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    ApproveChecked {
                                                        to: ctx.accounts.#to_acc_ident.to_account_info(),
                                                        mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                        delegate: ctx.accounts.#delegate_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                approve_checked(cpi_ctx, #amount)?;
                                            })
                                        },
                                        "closeAccount" => {
                                            let acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let destination_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let acc_ident = Ident::new(&acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let destination_acc_ident = Ident::new(&destination_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    CloseAccount {
                                                        account: ctx.accounts.#acc_ident.to_account_info(),
                                                        destination: ctx.accounts.#destination_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                close_account(cpi_ctx)?;
                                            })
                                        },
                                        "freezeAccount" => {
                                            let acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let mint_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let acc_ident = Ident::new(&acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let mint_acc_ident = Ident::new(&mint_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    FreezeAccount {
                                                        account: ctx.accounts.#acc_ident.to_account_info(),
                                                        mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                freeze_account(cpi_ctx)?;
                                            })
                                        },
                                        "initializeAccount" => {
                                            let acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let mint_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let acc_ident = Ident::new(&acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let mint_acc_ident = Ident::new(&mint_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    InitializeAccount3 {
                                                        account: ctx.accounts.#acc_ident.to_account_info(),
                                                        mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                initialize_account3(cpi_ctx)?;
                                            })
                                        },
                                        "revoke" => {
                                            let source_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let source_acc_ident = Ident::new(&source_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    Revoke {
                                                        source: ctx.accounts.#source_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                revoke(cpi_ctx)?;
                                            })
                                        },
                                        "syncNative" => {
                                            let acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let acc_ident = Ident::new(&acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    SyncNative {
                                                        account: ctx.accounts.#acc_ident.to_account_info(),
                                                    },
                                                );

                                                sync_native(cpi_ctx)?;
                                            })
                                        },
                                        "thawAccount" => {
                                            let acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let mint_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let acc_ident = Ident::new(&acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let mint_acc_ident = Ident::new(&mint_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());

                                            ix_body.push(quote!{
                                                let cpi_ctx = CpiContext::new(
                                                    ctx.accounts.token_program.to_account_info(),
                                                    ThawAccount {
                                                        account: ctx.accounts.#acc_ident.to_account_info(),
                                                        mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                        authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                    },
                                                );

                                                thaw_account(cpi_ctx)?;
                                            })
                                        },
                                        "transferChecked" => {
                                            let from_acc = c.args[0].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let mint_acc = c.args[1].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let to_acc = c.args[2].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let auth_acc = c.args[3].expr.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let from_acc_ident = Ident::new(&from_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let mint_acc_ident = Ident::new(&mint_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let to_acc_ident = Ident::new(&to_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let auth_acc_ident = Ident::new(&auth_acc.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let amount_expr = &c.args[4].expr;
                                            let amount = ProgramInstruction::get_amount_from_ts_arg(amount_expr)?;
                                            if let Some(cur_ix_acc) = ix_accounts.get(from_acc){
                                                if cur_ix_acc.seeds.is_some() {
                                                    ix_body.push(quote!{
                                                        let cpi_accounts = TransferChecked {
                                                            from: ctx.accounts.#from_acc_ident.to_account_info(),
                                                            mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                            to: ctx.accounts.#to_acc_ident.to_account_info(),
                                                            authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                        };
                                                        let signer_seeds = &[
                                                            &b"auth"[..],
                                                            &[ctx.accounts.escrow.auth_bump],
                                                        ];
                                                        let binding = [&signer_seeds[..]];
                                                        let ctx = CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(), cpi_accounts, &binding);
                                                        transfer_checked(ctx, #amount)?;
                                                    });
                                                } else {
                                                    ix_body.push(quote!{
                                                        let cpi_accounts = TransferChecked {
                                                            from: ctx.accounts.#from_acc_ident.to_account_info(),
                                                            mint: ctx.accounts.#mint_acc_ident.to_account_info(),
                                                            to: ctx.accounts.#to_acc_ident.to_account_info(),
                                                            authority: ctx.accounts.#auth_acc_ident.to_account_info(),
                                                        };
                                                        let cpi_ctx = CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts);
                                                        transfer_checked(cpi_ctx, #amount)?;
                                                    })
                                                }
                                            }
                                        },
                                        _ => {}
                                    }
                                }
                            }
                            Expr::Assign(a) => {
                                // let op = a.op;
                                let left_members = a.left.as_expr().ok_or(PoseidonError::ExprNotFound)?.as_member().ok_or(PoseidonError::MemberNotFound)?;
                                let left_obj = left_members.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                let left_prop = left_members.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                if ix_accounts.contains_key(left_obj){
                                    let left_obj_ident = Ident::new(&left_obj.to_case(Case::Snake), proc_macro2::Span::call_site());
                                    let left_prop_ident = Ident::new(&left_prop.to_case(Case::Snake), proc_macro2::Span::call_site());
                                    let cur_acc = ix_accounts.get_mut(left_obj).unwrap();
                                    cur_acc.is_mut = true;
                                    match *(a.clone().right) {
                                        Expr::New(exp) => {
                                            let right_lit  = exp.args.ok_or(anyhow!("need some value in  new expression"))?[0].expr.clone().expect_lit();
                                            let _lit_type = exp.callee.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            match right_lit {
                                                Lit::Num(num) => {
                                                    // match lit_type {
                                                    //     TsType::I64 => {
                                                    //     }
                                                    // }
                                                    let value = Literal::i64_unsuffixed(num.value as i64);
                                                    ix_body.push(quote!{
                                                        ctx.accounts.#left_obj_ident.#left_prop_ident =  #value;
                                                    });
                                                }
                                                _ => {}
                                            }
                                        },
                                        Expr::Ident(right_swc_ident) => {
                                            let right_ident = Ident::new(&right_swc_ident.sym.as_ref().to_case(Case::Snake), proc_macro2::Span::call_site());
                                            ix_body.push(quote!{
                                                ctx.accounts.#left_obj_ident.#left_prop_ident = #right_ident;
                                            });
                                        },
                                        Expr::Call(CallExpr { span: _, callee, args, type_args: _ }) => {
                                            let memebers = callee.as_expr().ok_or(PoseidonError::ExprNotFound)?.as_member().ok_or(PoseidonError::MemberNotFound).cloned()?;
                                            let prop: &str = &memebers.prop.as_ident().ok_or(anyhow!("expected a prop"))?.sym.as_ref();
                                            match *memebers.obj {
                                                Expr::Member(sub_members) => {
                                                    let sub_prop = sub_members.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                                    let sub_obj = sub_members.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                                    let right_sub_obj_ident = Ident::new(&sub_obj.to_case(Case::Snake), proc_macro2::Span::call_site());
                                                    let right_sub_prop_ident = Ident::new(&sub_prop.to_case(Case::Snake), proc_macro2::Span::call_site());
                                                    match *(args[0].expr.clone()) {
                                                        Expr::Lit(Lit::Num(num)) => {
                                                            let value = Literal::i64_unsuffixed(num.value as i64);
                                                            match prop {
                                                                "add" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident + #value;
                                                                    });
                                                                },
                                                                "sub" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident - #value;
                                                                    });
                                                                },
                                                                "mul" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident * #value;
                                                                    });
                                                                },
                                                                "div" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident / #value;
                                                                    });
                                                                },
                                                                "eq" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident == #value;
                                                                    });
                                                                },
                                                                "neq" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident != #value;
                                                                    });
                                                                },
                                                                "lt" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident < #value;
                                                                    });
                                                                },
                                                                "lte" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident <= #value;
                                                                    });
                                                                },
                                                                "gt" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident > #value;
                                                                    });
                                                                },
                                                                "gte" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident >= #value;
                                                                    });
                                                                },
                                                                "toBytes" => {
                                                                    ix_body.push(quote!{
                                                                        ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.accounts.#right_sub_obj_ident.#right_sub_prop_ident.to_bytes();
                                                                    });
                                                                },
                                                                _ => {}
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                                Expr::Ident(right_obj) => {
                                                    let right_obj = right_obj.sym.as_ref();
                                                    let right_prop = memebers.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                                    if right_prop == "getBump" {
                                                        let right_obj_ident = Ident::new(&right_obj.to_case(Case::Snake), proc_macro2::Span::call_site());
                                                        ix_body.push(quote!{
                                                            ctx.accounts.#left_obj_ident.#left_prop_ident = ctx.bumps.#right_obj_ident;
                                                        })
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                        Expr::Member(m) => {
                                            let right_obj = m.obj.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let right_prop = m.prop.as_ident().ok_or(PoseidonError::IdentNotFound)?.sym.as_ref();
                                            let right_obj_ident = Ident::new(&right_obj.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            let right_prop_ident = Ident::new(&right_prop.to_case(Case::Snake), proc_macro2::Span::call_site());
                                            if let Some(_) = ix_accounts.get(right_obj){
                                                ix_body.push(quote!{
                                                    ctx.accounts.#left_obj_ident.#left_prop_ident =  ctx.accounts.#right_obj_ident.key();
                                                });
                                            } else {
                                                ix_body.push(quote!{
                                                    ctx.accounts.#left_obj_ident.#left_prop_ident =  ctx.accounts.#right_obj_ident.#right_prop_ident;
                                                });
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            _ => {}
                        }
                    },
                    Stmt::Decl(_d) => {
                        // let kind  = d.clone().expect_var().kind;
                        // let decls = &d.clone().expect_var().decls[0];
                        // let name = decls.name.clone().expect_ident().id.sym.to_string().to_case(Case::Snake);
                        // let of_type = decls.name.clone().expect_ident().type_ann.expect("declaration stmt type issue").type_ann.expect_ts_type_ref().type_name.expect_ident().sym.to_string();
                        // if of_type == "Seeds" {
                        //     let elems = decls.init.clone().expect("declaration stmt init issue").expect_array().elems;
                        //     for elem in elems {
                        //         if let Some(seed) = elem {

                        //         }
                        //     }
                        // }
                    }
                    _ => {}
                }
                Ok(())
            }).collect::<Result<Vec<()>>>()?;

        ix.accounts = ix_accounts.into_values().collect();
        ix.body = ix_body;

        Ok(ix)
    }

    pub fn to_tokens(&self) -> TokenStream {
        let name = Ident::new(
            &self.name.to_case(Case::Snake),
            proc_macro2::Span::call_site(),
        );
        let ctx_name = Ident::new(
            &format!("{}Context", &self.name.to_case(Case::Pascal)),
            proc_macro2::Span::call_site(),
        );
        let args: Vec<TokenStream> = self
            .args
            .iter()
            .map(|a| {
                let name = Ident::new(&a.name, proc_macro2::Span::call_site());
                let of_type = &a.of_type;
                quote! { #name: #of_type }
            })
            .collect();
        let body = self.body.clone();
        let stmts = quote! {#(#body)*};
        quote! {
            pub fn #name (ctx: Context<#ctx_name>, #(#args)*) -> Result<()> {
                #stmts
                Ok(())

            }
        }
    }

    pub fn accounts_to_tokens(&self) -> TokenStream {
        let ctx_name = Ident::new(
            &format!("{}Context", &self.name.to_case(Case::Pascal)),
            proc_macro2::Span::call_site(),
        );
        let ix_a = &self.instruction_attributes;
        let s = &ix_a.token_streams;

        let mut accounts: Vec<TokenStream> = self.accounts.iter().map(|a| a.to_tokens(&ix_a.string_idents, &self.realloc)).collect();

        let ix_attributes = quote! {
            #[instruction(#(#s)*)]
        };
        if self.uses_associated_token_program {
            accounts.push(quote! {
                pub associated_token_program: Program<'info, AssociatedToken>,
            })
        }
        if self.uses_token_program {
            accounts.push(quote! {
                pub token_program: Program<'info, Token>,
            })
        }
        if self.uses_system_program {
            accounts.push(quote! {
                pub system_program: Program<'info, System>,
            })
        }


        println!("---------------------------------");
        println!("{:?}", ix_attributes);
        println!("---------------------------------");

        quote! {
            #[derive(Accounts)]
            #ix_attributes
            pub struct #ctx_name<'info> {
                #(#accounts)*
            }
        }
    }
}

fn extract_kind_str(keyword_type: TsKeywordTypeKind) -> Result<String, Error> {
    match keyword_type {
        TsKeywordTypeKind::TsStringKeyword => Ok("String".to_string()),
        _ => Err(PoseidonError::KeyWordTypeNotSupported(format!("{:?}", keyword_type)).into()),
    }
}

fn extract_of_type(binding: Box<swc_ecma_ast::TsTypeAnn>) -> Result<(String, bool), Error> {
    match binding.type_ann.as_ref() {
        TsType::TsTypeRef(_) => {
            let ident = binding
                .type_ann
                .expect_ts_type_ref()
                .type_name
                .expect_ident();

            Ok((ident.sym.to_string(), ident.optional))
        }
        TsType::TsArrayType(_) => {
            let keyword_type = binding
                .type_ann
                .expect_ts_array_type()
                .elem_type
                .expect_ts_keyword_type()
                .kind;

            let kind_type = extract_kind_str(keyword_type)
                .unwrap_or_else(|_| panic!("Keyword type {:?} is not supported", keyword_type));

            Ok((format!("Vec<{}>", kind_type), false))
        }
        TsType::TsKeywordType(_) => {
            let keyword_type = binding
                .type_ann
                .expect_ts_keyword_type()
                .kind;

            let kind_type = extract_kind_str(keyword_type)
                .unwrap_or_else(|_| panic!("Keyword type {:?} is not supported", keyword_type));

            Ok((kind_type, false))
        }
        _ => Err(PoseidonError::KeyWordTypeNotSupported(format!("{:?}", binding.type_ann.as_ref())).into()),
    }
}

#[derive(Debug, Clone)]
pub struct ProgramAccountField {
    pub name: String,
    pub of_type: String,
}

#[derive(Debug, Clone)]
pub struct ProgramAccount {
    pub name: String,
    pub fields: Vec<ProgramAccountField>,
    pub space: u16,
}

impl ProgramAccount {
    pub fn from_ts_expr(interface: TsInterfaceDecl) -> Self {
        match interface.extends.first() {
            Some(TsExprWithTypeArgs { expr, .. })
                if expr.clone().ident().is_some()
                    && expr.clone().ident().unwrap().sym == "Account" => {}
            _ => panic!("Custom accounts must extend Account type"),
        }
        let name: String = interface.id.sym.to_string();
        let mut space: u16 = 8; // anchor discriminator
        let fields: Vec<ProgramAccountField> = interface
            .body
            .body
            .iter()
            .map(|f| {
                let field = f.clone().ts_property_signature().expect("Invalid property");
                let field_name = field.key.ident().expect("Invalid property").sym.to_string();
                let binding = field.type_ann.expect("Invalid type annotation");
                let (field_type, _optional) = extract_of_type(binding)
                    .unwrap_or_else(|_| panic!("Keyword type is not supported"));

                // Reference https://book.anchor-lang.com/anchor_references/space.html
                match field_type.as_str() {
                    "Pubkey" => {
                        space += 32;
                    }
                    "u64" | "i64" => {
                        space += 8;
                    }
                    "u32" | "i32" => {
                        space += 4;
                    }
                    "u16" | "i16" => {
                        space += 2;
                    }
                    "u8" | "i8" => {
                        space += 1;
                    }
                    "string" | "String" | "Vec<string>" | "Vec<String>" => {
                        space += 4; // initial 4 bytes for string types
                    }
                    _ => {}
                }
                ProgramAccountField {
                    name: field_name,
                    of_type: field_type.to_string(),
                }
            })
            .collect();
        Self {
            name,
            fields,
            space,
        }
    }

    pub fn to_tokens(&self) -> TokenStream {
        let struct_name = Ident::new(&self.name, proc_macro2::Span::call_site());

        let fields: Vec<_> = self
            .fields
            .iter()
            .map(|field| {
                let field_name = Ident::new(
                    &field.name.to_case(Case::Snake),
                    proc_macro2::Span::call_site(),
                );

                let field_type = struct_rs_type_from_str(&field.of_type)
                        .unwrap_or_else(|_| panic!("Invalid type: {}", field.of_type));

                quote! { pub #field_name: #field_type }
            })
            .collect();

        quote! {
            #[account]
            pub struct #struct_name {
                #(#fields),*
            }
        }
    }
}

type SubMember = HashMap<String, Option<String>>; // submember_name : alias
type Member = HashMap<String, SubMember>; // member_name : submembers
type ProgramImport = HashMap<String, Member>; // src_pkg : members
pub struct ProgramModule {
    pub id: String,
    pub name: String,
    pub custom_types: HashMap<String, ProgramAccount>,
    pub instructions: Vec<ProgramInstruction>,
    pub accounts: Vec<ProgramAccount>,
    pub imports: ProgramImport,
}

impl ProgramModule {
    pub fn new() -> Self {
        Self {
            id: "Poseidon11111111111111111111111111111111111".to_string(),
            name: "AnchorProgram".to_string(),
            custom_types: HashMap::new(),
            instructions: vec![],
            accounts: vec![],
            imports: HashMap::new(),
        }
    }
    pub fn add_import(&mut self, src_pkg: &str, member_name: &str, sub_member_name: &str) {
        let mut alias: Option<String> = None;
        if sub_member_name == "Transfer" && member_name == "token" {
            alias = Some("TransferSPL".to_string());
        }
        if sub_member_name == "transfer" && member_name == "token" {
            alias = Some("transfer_spl".to_string());
        }
        if let Some(members) = self.imports.get_mut(src_pkg) {
            if !members.contains_key(member_name) {
                members.insert(
                    member_name.to_string(),
                    SubMember::from([(sub_member_name.to_string(), alias)]),
                );
            } else if let Some(submembers) = members.get_mut(member_name) {
                if !submembers.contains_key(sub_member_name) {
                    submembers.insert(sub_member_name.to_string(), alias);
                }
            }
        } else {
            self.imports.insert(
                src_pkg.to_string(),
                Member::from([(
                    member_name.to_string(),
                    SubMember::from([(sub_member_name.to_string(), alias)]),
                )]),
            );
        }
    }

    pub fn populate_from_class_expr(
        &mut self,
        class: &ClassExpr,
        custom_accounts: &HashMap<String, ProgramAccount>,
    ) -> Result<()> {
        self.name = class
            .ident
            .clone()
            .expect("Expected ident")
            .as_ref()
            .split("#")
            .next()
            .expect("Expected program to have a valid name")
            .to_string();
        let class_members = &class.class.body;
        let _ = class_members
            .iter()
            .map(|c| {
                match c.as_class_prop() {
                    Some(c) => {
                        // Handle as a class prop
                        if c.key.as_ident().expect("Invalid class property").sym == "PROGRAM_ID" {
                            let val = c
                                .value
                                .as_ref()
                                .expect("Invalid program ID")
                                .as_new()
                                .expect("Invalid program ID");
                            assert!(
                                val.callee.clone().expect_ident().sym == "Pubkey",
                                "Invalid program ID, expected new Pubkey(\"11111111111111.....\")"
                            );
                            self.id = match val.args.clone().expect("Invalid program ID")[0]
                                .expr
                                .clone()
                                .lit()
                                .expect("Invalid program ID")
                            {
                                Lit::Str(s) => s.value.to_string(),
                                _ => panic!("Invalid program ID"),
                            };
                        } else {
                            panic!("Invalid declaration")
                        }
                    }
                    None => match c.as_method() {
                        Some(c) => {
                            let ix =
                                ProgramInstruction::from_class_method(self, c, custom_accounts)
                                    .map_err(|e| anyhow!(e.to_string()))?;
                            self.instructions.push(ix);
                        }
                        None => panic!("Invalid class property or member"),
                    },
                }
                Ok(())
            })
            .collect::<Result<Vec<()>>>();
        Ok(())
    }

    pub fn to_tokens(&self) -> Result<TokenStream> {
        let program_name = Ident::new(
            &self.name.to_case(Case::Snake),
            proc_macro2::Span::call_site(),
        );
        let program_id = Literal::string(&self.id);
        let serialized_instructions: Vec<TokenStream> =
            self.instructions.iter().map(|x| x.to_tokens()).collect();
        let serialized_account_structs: Vec<TokenStream> = self
            .instructions
            .iter()
            .map(|x| x.accounts_to_tokens())
            .collect();

        let imports: TokenStream = match !self.imports.is_empty() {
            true => {
                let mut imports_vec: Vec<TokenStream> = vec![];
                for (src_pkg, members) in self.imports.iter() {
                    let src_pkg_ident = Ident::new(src_pkg, proc_macro2::Span::call_site());

                    let mut member_tokens: Vec<TokenStream> = vec![];
                    for (member_name, sub_members) in members.iter() {
                        let member_name_ident =
                            Ident::new(member_name, proc_macro2::Span::call_site());
                        let mut sub_member_tokens: Vec<TokenStream> = vec![];
                        for (sub_member_name, alias) in sub_members {
                            let sub_member_name_ident =
                                Ident::new(sub_member_name, proc_macro2::Span::call_site());
                            if alias.is_none() {
                                sub_member_tokens.push(quote! {#sub_member_name_ident});
                            } else {
                                let alias_str =
                                    alias.to_owned().ok_or(anyhow!("invalid alias in import"))?;
                                let alias_ident =
                                    Ident::new(&alias_str, proc_macro2::Span::call_site());
                                sub_member_tokens
                                    .push(quote! {#sub_member_name_ident as #alias_ident});
                            }
                        }

                        member_tokens.push(quote!(#member_name_ident :: {#(#sub_member_tokens),*}))
                    }
                    imports_vec.push(quote! {use #src_pkg_ident :: {#(#member_tokens),*};});
                }

                quote! {#(#imports_vec),*}
            }
            false => {
                quote!()
            }
        };
        let serialized_accounts: Vec<TokenStream> =
            self.accounts.iter().map(|x| x.to_tokens()).collect();
        let program = quote! {
            use anchor_lang::prelude::*;
            #imports
            declare_id!(#program_id);

            #[program]
            pub mod #program_name {
                use super::*;

                #(#serialized_instructions)*
            }

            #(#serialized_account_structs)*

            #(#serialized_accounts)*
        };
        Ok(program)
    }
}
