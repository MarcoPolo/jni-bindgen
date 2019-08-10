use super::*;
use class_file_visitor::method::*;

use std::io;

pub struct Method {
    access_flags:   MethodAccessFlags,
    java_class:     String,
    java_name:      String,
    java_desc:      String,
    rust_name:      Option<String>,
    mangling_style: MethodManglingStyle,
}

impl Method {
    pub fn new(context: &Context, class: &Class, method: MethodRef) -> Self {
        let mut result = Self {
            access_flags:   method.access_flags(),
            java_class:     class.this_class().name().to_owned(),
            java_name:      method.name().to_owned(),
            java_desc:      method.descriptor().to_owned(),
            rust_name:      None,
            mangling_style: MethodManglingStyle::Java, // Immediately overwritten bellow
        };
        result.set_mangling_style(context.config.codegen.method_naming_style); // rust_name + mangling_style
        result
    }

    pub fn rust_name(&self) -> Option<&str> {
        self.rust_name.as_ref().map(|s| s.as_str())
    }

    pub fn is_constructor(&self) -> bool { self.java_name == "<init>" }
    pub fn is_static_init(&self) -> bool { self.java_name == "<clinit>" }

    pub fn is_public(&self)     -> bool { self.access_flags.contains(MethodAccessFlags::PUBLIC)     }
    pub fn is_protected(&self)  -> bool { self.access_flags.contains(MethodAccessFlags::PROTECTED)  }
    pub fn is_private(&self)    -> bool { !(self.is_public() || self.is_protected()) }
    pub fn is_static(&self)     -> bool { self.access_flags.contains(MethodAccessFlags::STATIC)     }
    pub fn is_varargs(&self)    -> bool { self.access_flags.contains(MethodAccessFlags::VARARGS)    }
    pub fn is_bridge(&self)     -> bool { self.access_flags.contains(MethodAccessFlags::BRIDGE)     }
    // Ignored: FINAL | SYNCRONIZED | NATIVE | ABSTRACT | STRICT | SYNTHETIC

    pub fn mangling_style(&self) -> MethodManglingStyle { self.mangling_style }

    pub fn set_mangling_style(&mut self, style: MethodManglingStyle) {
        self.mangling_style = style;
        self.rust_name = if let Ok(name) = self.mangling_style.mangle(self.java_name.as_str(), self.java_desc.as_str()) {
            Some(name)
        } else {
            None // Failed to mangle
        };
    }

    pub fn emit(&self, context: &Context, indent: &str, out: &mut impl io::Write) -> io::Result<()> {
        let mut emit_reject_reasons = Vec::new();

        let constructor = self.java_name == "<init>";
        let static_init = self.java_name == "<clinit>";
        let public      = self.access_flags.contains(MethodAccessFlags::PUBLIC);
        let protected   = self.access_flags.contains(MethodAccessFlags::PROTECTED);
        let static_     = self.access_flags.contains(MethodAccessFlags::STATIC);
        let varargs     = self.access_flags.contains(MethodAccessFlags::VARARGS);
        let bridge      = self.access_flags.contains(MethodAccessFlags::BRIDGE);
        // Ignored: FINAL | SYNCRONIZED | NATIVE | ABSTRACT | STRICT | SYNTHETIC
        let _private    = !public && !protected;
        let _access     = if public { "public" } else if protected { "protected" } else { "private" };

        let java_class              = format!("{}", &self.java_class);
        let java_class_method       = format!("{}\x1f{}", &self.java_class, &self.java_name);
        let java_class_method_sig   = format!("{}\x1f{}\x1f{}", &self.java_class, &self.java_name, &self.java_desc);

        let ignored =
            context.config.ignore_classes          .contains(&java_class) ||
            context.config.ignore_class_methods    .contains(&java_class_method) ||
            context.config.ignore_class_method_sigs.contains(&java_class_method_sig);

        let renamed_to = context.config.rename_classes          .get(&java_class)
            .or_else(||  context.config.rename_class_methods    .get(&java_class_method))
            .or_else(||  context.config.rename_class_method_sigs.get(&java_class_method_sig));

        let descriptor = &self.java_desc;

        let method_name = if let Some(renamed_to) = renamed_to {
            renamed_to.clone()
        } else if let Some(name) = self.rust_name() {
            name.to_owned()
        } else {
            emit_reject_reasons.push("Failed to mangle method name");
            self.java_name.to_owned()
        };

        if !public      { emit_reject_reasons.push("Non-public method"); }
        if varargs      { emit_reject_reasons.push("Marked as varargs - haven't decided on how I want to handle this."); }
        if bridge       { emit_reject_reasons.push("Bridge method - type erasure"); }
        if static_init  { emit_reject_reasons.push("Static class constructor - never needs to be called by Rust."); }
        if ignored      { emit_reject_reasons.push("[[ignore]]d"); }

        // Parameter names may or may not be available as extra debug information.  Example:
        // https://docs.oracle.com/javase/tutorial/reflect/member/methodparameterreflection.html

        let mut params_array = String::new(); // Contents of let __jni_args = [...];
        let mut ret_decl = String::new();     // Contents of fn name<'env>() -> Result<...> {
        let mut ret_method_fragment = "";     // Contents of Call...MethodA
        let descriptor = if let Ok(d) = JniDescriptor::new(descriptor) {
            d
        } else {
            emit_reject_reasons.push("Invalid method descriptor");
            JniDescriptor::new("()V").unwrap()
        };

        // Contents of fn name<'env>(...) {
        let mut params_decl = if constructor || static_ {
            match context.config.codegen.static_env {
                config::toml::StaticEnvStyle::Explicit => String::from("__jni_env: &'env __jni_bindgen::Env"),
                config::toml::StaticEnvStyle::Implicit => {
                    emit_reject_reasons.push("StaticEnvStyle::Implicit not yet implemented");
                    String::new()
                },
                config::toml::StaticEnvStyle::__NonExhaustive => {
                    emit_reject_reasons.push("StaticEnvStyle::__NonExhaustive is invalid, silly goose!");
                    String::new()
                },
            }
        } else {
            String::from("&'env self")
        };

        for (arg_idx, segment) in descriptor.enumerate() {
            match segment {
                JniDescriptorSegment::Parameter(parameter) => {
                    let arg_name = format!("arg{}", arg_idx);

                    let mut param_is_object = false; // XXX

                    let arg_type = match parameter {
                        JniField::Single(JniBasicType::Void) => {
                            emit_reject_reasons.push("Void arguments aren't a thing");
                            "???".to_owned()
                        },
                        JniField::Single(JniBasicType::Boolean)     => "bool".to_owned(),
                        JniField::Single(JniBasicType::Byte)        => "i8".to_owned(),
                        JniField::Single(JniBasicType::Char)        => "__jni_bindgen::jchar".to_owned(),
                        JniField::Single(JniBasicType::Short)       => "i16".to_owned(),
                        JniField::Single(JniBasicType::Int)         => "i32".to_owned(),
                        JniField::Single(JniBasicType::Long)        => "i64".to_owned(),
                        JniField::Single(JniBasicType::Float)       => "f32".to_owned(),
                        JniField::Single(JniBasicType::Double)      => "f64".to_owned(),
                        JniField::Single(JniBasicType::Class(class)) => {
                            param_is_object = true;
                            match context.java_to_rust_path(class) {
                                Ok(path) => format!("impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env {}>>", path),
                                Err(_) => {
                                    emit_reject_reasons.push("Failed to resolve JNI path to Rust path for argument type");
                                    format!("{:?}", class)
                                }
                            }
                        },
                        JniField::Array { levels: 1, inner: JniBasicType::Void      } => {
                            emit_reject_reasons.push("Accepting arrays of void isn't a thing");
                            "???".to_owned()
                        }
                        JniField::Array { levels: 1, inner: JniBasicType::Boolean   } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::BooleanArray>>".to_owned() },
                        JniField::Array { levels: 1, inner: JniBasicType::Byte      } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::ByteArray   >>".to_owned() },
                        JniField::Array { levels: 1, inner: JniBasicType::Char      } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::CharArray   >>".to_owned() },
                        JniField::Array { levels: 1, inner: JniBasicType::Short     } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::ShortArray  >>".to_owned() },
                        JniField::Array { levels: 1, inner: JniBasicType::Int       } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::IntArray    >>".to_owned() },
                        JniField::Array { levels: 1, inner: JniBasicType::Long      } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::LongArray   >>".to_owned() },
                        JniField::Array { levels: 1, inner: JniBasicType::Float     } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::FloatArray  >>".to_owned() },
                        JniField::Array { levels: 1, inner: JniBasicType::Double    } => { param_is_object = true; "impl __jni_bindgen::std::convert::Into<__jni_bindgen::std::option::Option<&'env __jni_bindgen::DoubleArray >>".to_owned() },
                        JniField::Array { .. } => {
                            param_is_object = true;
                            emit_reject_reasons.push("Passing in arrays of arrays/objects is not yet supported");
                            format!("{:?}", parameter)
                        },
                    };

                    if !params_array.is_empty() {
                        params_array.push_str(", ");
                    }

                    params_array.push_str("__jni_bindgen::AsJValue::as_jvalue(");
                    params_array.push_str("&");
                    params_array.push_str(arg_name.as_str());
                    if param_is_object { params_array.push_str(".into()"); }
                    params_array.push_str(")");

                    if !params_decl.is_empty() {
                        params_decl.push_str(", ");
                    }

                    params_decl.push_str(arg_name.as_str());
                    params_decl.push_str(": ");
                    params_decl.push_str(arg_type.as_str());
                },
                JniDescriptorSegment::Return(ret) => {
                    ret_decl = match ret {
                        JniField::Single(JniBasicType::Void)        => "()".to_owned(),
                        JniField::Single(JniBasicType::Boolean)     => "bool".to_owned(),
                        JniField::Single(JniBasicType::Byte)        => "i8".to_owned(),
                        JniField::Single(JniBasicType::Char)        => "__jni_bindgen::jchar".to_owned(),
                        JniField::Single(JniBasicType::Short)       => "i16".to_owned(),
                        JniField::Single(JniBasicType::Int)         => "i32".to_owned(),
                        JniField::Single(JniBasicType::Long)        => "i64".to_owned(),
                        JniField::Single(JniBasicType::Float)       => "f32".to_owned(),
                        JniField::Single(JniBasicType::Double)      => "f64".to_owned(),
                        JniField::Single(JniBasicType::Class(class)) => {
                            match context.java_to_rust_path(class) {
                                Ok(path) => format!("__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, {}>>", path),
                                Err(_) => {
                                    emit_reject_reasons.push("Failed to resolve JNI path to Rust path for return type");
                                    format!("{:?}", class)
                                },
                            }
                        },
                        JniField::Array { levels: 1, inner: JniBasicType::Void      } => {
                            emit_reject_reasons.push("Returning arrays of void isn't a thing");
                            "???".to_owned()
                        }
                        JniField::Array { levels: 1, inner: JniBasicType::Boolean   } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::BooleanArray>>".to_owned(),
                        JniField::Array { levels: 1, inner: JniBasicType::Byte      } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::ByteArray   >>".to_owned(),
                        JniField::Array { levels: 1, inner: JniBasicType::Char      } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::CharArray   >>".to_owned(),
                        JniField::Array { levels: 1, inner: JniBasicType::Short     } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::ShortArray  >>".to_owned(),
                        JniField::Array { levels: 1, inner: JniBasicType::Int       } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::IntArray    >>".to_owned(),
                        JniField::Array { levels: 1, inner: JniBasicType::Long      } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::LongArray   >>".to_owned(),
                        JniField::Array { levels: 1, inner: JniBasicType::Float     } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::FloatArray  >>".to_owned(),
                        JniField::Array { levels: 1, inner: JniBasicType::Double    } => "__jni_bindgen::std::option::Option<__jni_bindgen::Local<'env, __jni_bindgen::DoubleArray >>".to_owned(),
                        JniField::Array { .. } => {
                            emit_reject_reasons.push("Returning arrays of objects or arrays not yet supported");
                            format!("{:?}", ret)
                        }
                    };

                    ret_method_fragment = match ret {
                        JniField::Single(JniBasicType::Void)        => "void",
                        JniField::Single(JniBasicType::Boolean)     => "boolean",
                        JniField::Single(JniBasicType::Byte)        => "byte",
                        JniField::Single(JniBasicType::Char)        => "char",
                        JniField::Single(JniBasicType::Short)       => "short",
                        JniField::Single(JniBasicType::Int)         => "int",
                        JniField::Single(JniBasicType::Long)        => "long",
                        JniField::Single(JniBasicType::Float)       => "float",
                        JniField::Single(JniBasicType::Double)      => "double",
                        JniField::Single(JniBasicType::Class(_))    => "object",
                        JniField::Array { .. }                      => "object",
                    };
                },
            }
        }

        if constructor {
            if ret_method_fragment == "void" {
                ret_method_fragment = "object";
                ret_decl = match context.java_to_rust_path(self.java_class.as_str()) {
                    Ok(path) => format!("__jni_bindgen::Local<'env, {}>", path),
                    Err(_) => {
                        emit_reject_reasons.push("Failed to resolve JNI path to Rust path for this type");
                        format!("{:?}", self.java_class.as_str())
                    },
                };
            } else {
                emit_reject_reasons.push("Constructor should've returned void");
            }
        }

        let indent = if !emit_reject_reasons.is_empty() {
            format!("{}        // ", indent)
        } else {
            format!("{}        ", indent)
        };
        let access = if public { "pub " } else { "" };
        writeln!(out, "")?;
        for reason in &emit_reject_reasons {
            writeln!(out, "{}// Not emitting: {}", indent, reason)?;
        }
        if let Some(url) = KnownDocsUrl::from_method(context, self.java_class.as_str(), self.java_name.as_str(), self.java_desc.as_str()) {
            writeln!(out, "{}/// [{}]({})", indent, url.label, url.url)?;
        }
        writeln!(out, "{}{}fn {}<'env>({}) -> __jni_bindgen::Result<{}> {{", indent, access, method_name, params_decl, ret_decl)?;
        writeln!(out, "{}    // class.name() == {:?}, method.access_flags() == {:?}, .name() == {:?}, .descriptor() == {:?}", indent, &self.java_class, self.access_flags, &self.java_name, &self.java_desc)?;
        writeln!(out, "{}    unsafe {{", indent)?;
        writeln!(out, "{}        let __jni_args = [{}];", indent, params_array)?;
        if constructor || static_ {
            match context.config.codegen.static_env {
                config::toml::StaticEnvStyle::Explicit          => {},
                config::toml::StaticEnvStyle::Implicit          => writeln!(out, "{}    let __jni_env = ...?;", indent)?, // XXX
                config::toml::StaticEnvStyle::__NonExhaustive   => writeln!(out, "{}    let __jni_env = ...?;", indent)?, // XXX
            };
        } else {
            writeln!(out, "{}        let __jni_env = __jni_bindgen::Env::from_ptr(self.0.env);", indent)?;
        }

        writeln!(out, "{}        let (__jni_class, __jni_method) = __jni_env.require_class_{}method({}, {}, {});", indent, if static_ { "static_" } else { "" }, emit_cstr(self.java_class.as_str()), emit_cstr(self.java_name.as_str()), emit_cstr(self.java_desc.as_str()) )?;

        if constructor {
            writeln!(out, "{}        __jni_env.new_object_a(__jni_class, __jni_method, __jni_args.as_ptr())", indent)?;
        } else if static_ {
            writeln!(out, "{}        __jni_env.call_static_{}_method_a(__jni_class, __jni_method, __jni_args.as_ptr())", indent, ret_method_fragment)?;
        } else {
            writeln!(out, "{}        __jni_env.call_{}_method_a(self.0.object, __jni_method, __jni_args.as_ptr())", indent, ret_method_fragment)?;
        }
        writeln!(out, "{}    }}", indent)?;
        writeln!(out, "{}}}", indent)?;
        Ok(())
    }
}

fn emit_cstr(s: &str) -> String {
    let mut s = format!("{:?}", s); // XXX
    s.insert_str(s.len() - 1, "\\0");
    s
}