use super::*;

impl Context {
    /// Matching:   "unsafe ..."
    /// Goal:       "unsafe impl class some.java.ClassName { ... }"
    pub(super) fn on_unsafe(&mut self, input: &mut impl TokenIter) {
        if let Err(bad) = expect_keyword(input.next(), "impl") {
            self.error_at(&bad, "Expected:  impl class some.java.ClassName { ... }");
            skip(bad.as_ref(), input, ";}");
            return;
        }

        if let Err(bad) = expect_keyword(input.next(), "class") {
            self.error_at(&bad, "Expected:  class some.java.ClassName { ... }");
            skip(bad.as_ref(), input, ";}");
            return;
        }

        let class = match expect_java_ns_class(input) {
            Ok((prefix, class)) => format!("{}{}", prefix, class),
            Err(bad) => {
                self.error_at(&bad, "Expected:  some.java.ClassName { ... }");
                skip(bad.as_ref(), input, ";}");
                return;
            },
        };

        self.on_unsafe_impl_class_named(class, input)
    }

    /// Matching:   "unsafe impl class some.java.ClassName"
    /// Goal:       "unsafe impl class some.java.ClassName { ... }"
    fn on_unsafe_impl_class_named(&mut self, class_id: String, input: &mut impl TokenIter) {
        let impl_block = input.next();
        let impl_block = match impl_block {
            Some(TokenTree::Group(ref group)) if group.delimiter() == Delimiter::Brace => group,
            bad => {
                skip(bad.as_ref(), input, ";}");
                self.error_at(&bad, "Expected:  { ...java class impl block... }");
                return;
            },
        };

        let class_fqn = if let Some(c) = self.imports.get(&class_id) { c } else { &class_id };
        let class_jni = escape::java_fqn_class_name_to_c_identifier(class_fqn);

        let mut input = impl_block.stream().into_iter();
        let input = &mut input;
        while input.clone().next().is_some() {
            let annotations = self.parse_method_annotations(input);
            let return_type = Return::new(self.consume_resolved_java_identifier(input));

            // Parse method name
            let method_name = match input.next() {
                Some(TokenTree::Ident(i)) => i,
                bad => {
                    skip(bad.as_ref(), input, ";}");
                    self.error_at(&bad, "Expected:  javaMethodName");
                    continue;
                },
            };

            // Parse method arguments
            let arguments_list = input.next();
            let arguments_list = match arguments_list {
                Some(TokenTree::Group(g)) => match g.delimiter() {
                    Delimiter::Parenthesis => g,
                    _ => {
                        skip(None, input, ";}");
                        self.error_at(&g, "Expected:  (...java-style function arguments...)");
                        continue;
                    },
                },
                bad => {
                    self.error_at(&bad, "Expected:  (...java-style function arguments...)");
                    skip(bad.as_ref(), input, ";}");
                    continue;
                },
            };
            let arguments_list = self.parse_method_arguments_list(arguments_list, annotations.is_static).unwrap_or(vec![]);

            // Parse method body
            let function_impl = input.next();
            let function_impl = match function_impl {
                Some(TokenTree::Group(ref g)) if g.delimiter() == Delimiter::Brace => g,
                bad => {
                    self.error_at(&bad, "Expected:  {...rust-style function body...}");
                    skip(bad.as_ref(), input, ";}");
                    continue;
                },
            };



            // Emit method
            let this_or_class       = if annotations.is_static { quote!{class}  } else { quote!{this}    };
            let this_or_class_type  = if annotations.is_static { quote!{::jni_sys::jclass} } else { quote!{::jni_sys::jobject} };

            let mut outer_method = format!("Java_{}_{}", class_jni, method_name);
            if arguments_list.len() > 0 {
                outer_method.push_str("__");
                for arg in arguments_list.iter() {
                    outer_method.push_str(&arg.jni);
                }
            }
            let outer_method    = Ident::new(outer_method.as_str(),     method_name.span());
            let inner_method    = method_name.clone();

            let outer_return = &return_type.outer;
            let mut outer_args = TokenStream::from(quote! {
                env:            &::jni_glue::Env,
                #this_or_class: #this_or_class_type,
            });

            let inner_return = &return_type.inner;
            let inner_return = quote!{ std::result::Result<#inner_return, ::jni_glue::Throw> }; // XXX: Err should be something like Local<java::lang::Exception> or similar?
            let mut inner_args = TokenStream::from(quote! {
                env:            &'env ::jni_glue::Env,
                #this_or_class: #this_or_class_type, // XXX
            });

            let mut forward_args = TokenStream::from(quote! {
                env,
                #this_or_class, // XXX
            });

            for arg in arguments_list.iter() {
                let (outer, inner, name) = (&arg.outer, &arg.inner, &arg.name);
                outer_args  .extend(quote!{ #name: #outer, });
                inner_args  .extend(quote!{ #name: #inner, });
                forward_args.extend(quote!{ #name,         });
            }
 
            let method = quote! {
                #[doc(hidden)] #[no_mangle] pub unsafe extern "stdcall" fn #outer_method (#outer_args) -> #outer_return {
                    fn #inner_method<'env>(#inner_args) -> #inner_return {
                        #function_impl
                    }

                    let r = ::std::panic::catch_unwind(|| #inner_method(#forward_args));
                    env.unwrap_jni_glue_result(r)
                }
            };

            self.output.extend(method);
        }
    }

    fn consume_resolved_java_identifier(&self, input: &mut impl TokenIter) -> String {
        let id = consume_java_identifier(input);
        if let Some(id) = self.imports.get(&id) { id.clone() } else { id }
    }

    fn parse_method_annotations(&mut self, input: &mut impl TokenIter) -> MethodAnnotations {
        let mut annotations = MethodAnnotations::default();
        
        loop {
            let mut peek = input.clone();
            match peek.next() {
                Some(TokenTree::Punct(ref punct)) if punct.as_char() == '@' => {
                    *input = peek;
                    match input.next() {
                        Some(TokenTree::Ident(_keyword)) => {}, // Override or similar annotation
                        bad => {
                            self.error_at(&bad, "Expected:  @Annotation");
                            continue;
                        },
                    }
                },
                Some(TokenTree::Ident(keyword)) => {
                    match keyword.to_string().as_str() {
                        "static" if annotations.is_static => self.error_at(&keyword, "Duplicate static keyword"),
                        "static" => annotations.is_static = true,
                        "public" | "protected" | "private" | "final" | "native" => {}, // Ignored
                        _other => break, // Hopefully a method return type
                    }
                    *input = peek;
                },
                bad => {
                    self.error_at(&bad, "Expected:  Method keyword or return type");
                    *input = peek;
                },
            }
        }

        annotations
    }

    fn parse_method_arguments_list(&mut self, input: Group, is_static: bool) -> Result<Vec<Argument>,()> {
        debug_assert_eq!(input.delimiter(), Delimiter::Parenthesis);
        let mut input = input.stream().into_iter();
        let input = &mut input;

        // Expect   `&env`    or    `&'env env`
        match expect_punct_2(input.next(), "&") {
            Ok(_amp) => {},
            Err(bad) => return Err(self.error_at(&bad, "Expected:  &env")),
        };
        let _env_lifetime = match input.next() {
            Some(TokenTree::Punct(ref p)) if p.as_char() == '\'' => { // &'env env
                let env_lifetime = match expect_ident2(input.next()) {
                    Ok(lt) => lt,
                    Err(bad) => return Err(self.error_at(&bad, "Expected:  env lifetime for  `&'... env`")),
                };
                expect_keyword(input.next(), "env").map_err(|bad| self.error_at(&bad, &format!("Expected:  `env`  keyword of  `&'{} env`", env_lifetime)))?;
                Some(env_lifetime)
            },
            Some(TokenTree::Ident(ref ident)) if ident == "env" => None, // &env
            bad => return Err(self.error_at(&bad, "Expected:  &env or &'env env")),
        };
        match expect_punct_2(input.next(), ",") {
            Ok(_comma) => {},
            Err(bad) => return Err(self.error_at(&bad, "Expected:  ',' after &env")),
        };

        // Expect    this    or    class
        let expected_kw = if is_static { "class" } else { "this" };
        expect_ident_str(input.next(), expected_kw, |ident| if ident == expected_kw { Ok(()) } else { Err(()) });

        // 0 or more arguments
        let mut args = Vec::new();
        while !input.clone().next().is_none() {
            expect_punct(input.next(), ",");
            if input.clone().next().is_none() { break };

            let java_type = self.consume_resolved_java_identifier(input);
            let name = expect_ident(input.next(), "argument_name", |i| Ok(i.clone()));
            args.push(Argument::new(name, java_type));
        }

        Ok(args)
    }
}

#[derive(Default)]
struct MethodAnnotations {
    pub is_static:  bool,
}