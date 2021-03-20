use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Group, Literal, Punct, Spacing, Span};
use quote::{ToTokens, TokenStreamExt};
use regex::{Captures, Regex};
use shaderc::CompilationArtifact;
use std::{collections::HashMap, env, fs::File, io::Read, path::Path};
use syn::{parse::ParseStream, parse_macro_input, Error, Ident, LitStr, Token};
use ubo::{BindgenFieldType, BindgenStruct, ModuleType, StructRegistry};

mod ubo;

// input

struct DefinesInput {
    defines: Vec<(String, Option<String>)>,
}

struct ModuleInput {
    source: String,
    include_path: Option<String>,
    defines: DefinesInput,
    default_defines: bool,
    entry: Option<String>,
    debug: bool,
    span: Span,
}

struct PipelineInput {
    // name: String,
    modules: HashMap<ModuleType, ModuleInput>,
}

// processed

struct CompiledModule {
    spirv: CompilationArtifact,
    module_type: ModuleType,
}

struct Pipeline {
    // name: String,
    modules: HashMap<ModuleType, CompiledModule>,
    bindgen_structs: Vec<BindgenStruct>,
}

// imp input

impl std::ops::AddAssign for DefinesInput {
    fn add_assign(&mut self, rhs: Self) {
        let mut defines = rhs.defines;
        self.defines.append(&mut defines);
    }
}

impl syn::parse::Parse for DefinesInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut defines = Vec::new();

        while !input.is_empty() {
            let name: LitStr = input.parse()?;

            if input.is_empty() {
                defines.push((name.value(), None));
                break;
            }

            let punct: Punct = input.parse()?;
            match punct.as_char() {
                '=' => {
                    let value: LitStr = input.parse()?;
                    defines.push((name.value(), Some(value.value())));

                    if input.is_empty() {
                        break;
                    }

                    input.parse::<Token![,]>()?;
                }
                ',' => {
                    continue;
                }
                _ => {
                    return Err(Error::new(
                        punct.span(),
                        "Invalid punctuation, only '=' and ',' are valid",
                    ))
                }
            }
        }

        Ok(Self { defines })
    }
}

impl parse_macro_input::ParseMacroInput for PipelineInput
// impl syn::parse::Parse for PipelineInput
{
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut modules = HashMap::<ModuleType, ModuleInput>::new();

        while !input.is_empty() {
            let shader: Ident = input.parse()?;
            let shader_type_string = shader.to_string();

            input.parse::<Token![:]>()?;

            let group: Group = input.parse()?;
            let group_tokens: TokenStream = group.stream().into();
            let module_type = match shader_type_string.as_str() {
                "vs" | "vertex" | "vert" => ModuleType::Vertex,
                "fs" | "fragment" | "frag" => ModuleType::Fragment,
                _ => {
                    return Err(Error::new(
                        shader.span(),
                        format!("Unknown shader type: {}", shader_type_string),
                    ));
                }
            };

            if modules.contains_key(&module_type) {
                return Err(Error::new(shader.span(), "Duplicate shader module"));
            } else {
                modules.insert(module_type, syn::parse::<ModuleInput>(group_tokens)?);
            }
        }

        Ok(PipelineInput { modules })
    }
}

impl syn::parse::Parse for ModuleInput {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut end_span = input.span();
        let mut source = None;
        let mut include_path = None;
        let mut defines = DefinesInput {
            defines: Vec::new(),
        };
        let mut default_defines = true;
        let mut entry = None;
        let mut debug = false;

        while !input.is_empty() {
            let field_type: Ident = input.parse()?;
            let field_type_string = field_type.to_string();

            match field_type_string.as_str() {
                "p" | "path" => {
                    input.parse::<Token![:]>()?;

                    if source.is_some() {
                        return Err(Error::new(
                            field_type.span(),
                            "'source' or 'path' field already specified",
                        ));
                    }

                    let path: LitStr = input.parse()?;
                    end_span = path.span();
                    source = Some(read_shader_source(path.value(), path.span())?);

                    if include_path.is_none() {
                        let source_path_string = path.value();
                        let source_path = Path::new(source_path_string.as_str());
                        include_path = Some(
                            source_path
                                .parent()
                                .ok_or(Error::new(path.span(), "File does not have a directory"))?
                                .to_str()
                                .unwrap_or_else(|| panic!("Path unwrap failed"))
                                .into(),
                        );
                    }
                }
                "s" | "src" | "source" => {
                    input.parse::<Token![:]>()?;

                    if source.is_some() {
                        return Err(Error::new(
                            field_type.span(),
                            "'source' or 'path' field already specified",
                        ));
                    }

                    let source_lit: LitStr = input.parse()?;
                    end_span = source_lit.span();
                    source = Some(source_lit.value());
                }
                "i" | "inc" | "include" => {
                    input.parse::<Token![:]>()?;

                    let path: LitStr = input.parse()?;
                    end_span = path.span();
                    include_path = Some(path.value());
                }
                "d" | "def" | "define" => {
                    input.parse::<Token![:]>()?;

                    let group: Group = input.parse()?;
                    end_span = group.span();

                    let group_tokens: TokenStream = group.stream().into();
                    defines += syn::parse::<DefinesInput>(group_tokens)?;
                }
                "n" | "na" | "no-autodefine" => {
                    default_defines = false;
                }
                "e" | "ep" | "entry" => {
                    input.parse::<Token![:]>()?;

                    let ep: LitStr = input.parse()?;
                    end_span = ep.span();
                    entry = Some(ep.value());
                }
                "debug" => {
                    debug = true;
                }
                _ => {
                    return Err(Error::new(
                        field_type.span(),
                        format!("Invalid field '{}'", field_type_string),
                    ));
                }
            }
        }

        let source = source.ok_or(Error::new(
            end_span,
            "Missing shader source add either 'source' or 'path' field",
        ))?;

        Ok(Self {
            source,
            include_path,
            defines,
            default_defines,
            entry,
            debug,
            span: end_span,
        })
    }
}

fn read_shader_source(path: String, span: Span) -> syn::Result<String> {
    let root = // Span::call_site().source_file();
						env::var("CARGO_MANIFEST_DIR").unwrap_or(".".into());
    let root_path = Path::new(root.as_str());

    let full_path = root_path.join(path);

    if !full_path.is_file() {
        Err(Error::new(span, "File not found"))
    } else {
        let mut file = File::open(full_path).or(Err(Error::new(span, "Could not open file")))?;
        let mut buf = String::new();
        file.read_to_string(&mut buf)
            .or(Err(Error::new(span, "Could not read from file")))?;

        Ok(buf)
    }
}

// impl processed

fn glsl_attrib_macros<'a>(
    source: &'a str,
    module: ModuleType,
    struct_reg: &mut StructRegistry,
) -> (String, Vec<BindgenStruct>) {
    struct_reg.next_module();

    let comment_matcher = Regex::new(r#"(//.*)|(/\*(.|(\r?\n))*\*/)"#).unwrap();

    let attrib_matcher =
        Regex::new(r#"#\[gears_(bind)?(gen)\(.+\)\]((\r?\n)?.+)\{([^}]+)*(\r?\n)?\}.+;"#).unwrap();

    let mut bindgen_structs = Vec::new();
    let mut ident_renameres = Vec::new();

    let mut output = comment_matcher.replace_all(source, " ").to_string();

    output = attrib_matcher
        .replace_all(&output[..], |caps: &Captures| {
            let cap = &caps[0];
            match syn::parse_str::<BindgenStruct>(cap) {
                Ok(mut s) => {
                    s.meta.in_module = module;
                    s.generate(struct_reg);
                    let glsl = format!("\n{}", s.to_glsl());

                    // uniforms do not have to be renamed
                    match &s.meta.bind_type {
                        BindgenFieldType::Uniform(_) => (),
                        BindgenFieldType::In(_) | BindgenFieldType::Out(_) => {
                            ident_renameres.push(
                                Regex::new(format!("\\b{}\\.\\b", s.field_name).as_str()).unwrap(),
                            );
                        }
                    };

                    // bind only gears_bindgen not gears_gen for ex.
                    if s.meta.bind {
                        bindgen_structs.push(s);
                    }
                    glsl
                }
                Err(e) => {
                    panic!("attrib failed: {:?}, {}", e.to_string(), cap);
                }
            }
        })
        .to_string();

    for ident_renamer in ident_renameres {
        output = ident_renamer
            .replace_all(&output[..], |caps: &Captures| {
                let cap = &caps[0];
                let cap = &cap[..cap.len() - 1];
                format!("_{}_", cap)
            })
            .to_string();
    }

    (output, bindgen_structs)
}

impl Pipeline {
    fn new(input: PipelineInput) -> syn::Result<Self> {
        let mut struct_reg = StructRegistry::new();
        let mut bindgen_structs = Vec::new();
        let modules = input
            .modules
            .into_iter()
            .map(|(module_type, input)| {
                let span = input.span;
                let (source, mut new_bindgen_structs) =
                    glsl_attrib_macros(input.source.as_str(), module_type.clone(), &mut struct_reg);
                bindgen_structs.append(&mut new_bindgen_structs);

                let spirv = compile_shader_module(
                    module_type.kind(),
                    source.as_ref(),
                    module_type.name(),
                    input.entry.as_ref().map_or("main", |e| e.as_str()),
                    input
                        .include_path
                        .as_ref()
                        .map_or(None, |s| Some(Path::new(s))),
                    &input.defines,
                    input.default_defines,
                    input.debug,
                )
                .or_else(|err| Err(Error::new(span, err)))?;

                Ok((module_type.clone(), CompiledModule { spirv, module_type }))
            })
            .collect::<Result<HashMap<_, _>, Error>>()?;

        Ok(Pipeline {
            modules,
            bindgen_structs,
        })
    }
}

impl ModuleType {
    pub fn name(&self) -> &'static str {
        match self {
            ModuleType::Fragment => "FRAG",
            ModuleType::Vertex => "VERT",
        }
    }

    pub fn kind(&self) -> shaderc::ShaderKind {
        match self {
            ModuleType::Fragment => shaderc::ShaderKind::Fragment,
            ModuleType::Vertex => shaderc::ShaderKind::Vertex,
        }
    }
}

impl ToTokens for Pipeline {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        for (_, module) in self.modules.iter() {
            module.to_tokens(tokens);
        }

        for bindgen_struct in self.bindgen_structs.iter() {
            bindgen_struct.to_tokens(tokens);
        }
    }
}

impl ToTokens for CompiledModule {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.append(Ident::new("pub", Span::call_site()));
        tokens.append(Ident::new("const", Span::call_site()));
        tokens.append(Ident::new(
            format!("{}_SPIRV", self.module_type.name()).as_str(),
            Span::call_site(),
        ));

        tokens.append(Punct::new(':', Spacing::Alone));

        tokens.append(Punct::new('&', Spacing::Joint));
        let mut u8_token = proc_macro2::TokenStream::new();
        u8_token.append(Ident::new("u8", Span::call_site()));
        tokens.append(Group::new(Delimiter::Bracket, u8_token));

        tokens.append(Punct::new('=', Spacing::Joint));

        tokens.append(Punct::new('&', Spacing::Joint));
        let mut u8_list = proc_macro2::TokenStream::new();
        for &byte in self.spirv.as_binary_u8() {
            u8_list.append(Literal::u8_unsuffixed(byte));
            u8_list.append(Punct::new(',', Spacing::Alone));
        }
        tokens.append(Group::new(Delimiter::Bracket, u8_list));
        tokens.append(Punct::new(';', Spacing::Alone));
    }
}

static mut STATIC_COMPILER: Option<shaderc::Compiler> = None;

fn compile_shader_module(
    kind: shaderc::ShaderKind,
    source: &str,
    name: &str,
    entry: &str,
    include_path: Option<&Path>,
    defines: &DefinesInput,
    default_defines: bool,
    debug: bool,
) -> Result<shaderc::CompilationArtifact, String> {
    let compiler = unsafe {
        if STATIC_COMPILER.is_none() {
            STATIC_COMPILER = Some(
                shaderc::Compiler::new()
                    .unwrap_or_else(|| panic!("Could not create a shaderc Compiler")),
            );
            STATIC_COMPILER.as_mut().unwrap()
        } else {
            STATIC_COMPILER.as_mut().unwrap()
        }
    };

    let mut options = shaderc::CompileOptions::new()
        .unwrap_or_else(|| panic!("Could not create a shaderc CompileOptions"));
    options.set_optimization_level(shaderc::OptimizationLevel::Zero);
    options.set_include_callback(
        |name: &str, _include_type: shaderc::IncludeType, _source: &str, _depth: usize| {
            let full_path = include_path.ok_or("No include path")?.join(name);
            let mut file = File::open(&full_path).or(Err(format!(
                "Could not open file '{}'",
                full_path.to_str().ok_or("Path unwrap failed")?
            )))?;

            let mut content = String::new();
            file.read_to_string(&mut content).or(Err(format!(
                "Could not read from file '{}'",
                full_path.to_str().ok_or("Path unwrap failed")?
            )))?;

            Ok(shaderc::ResolvedInclude {
                content,
                resolved_name: String::from(
                    full_path
                        .to_str()
                        .unwrap_or_else(|| panic!("Path unwrap failed")),
                ),
            })
        },
    );

    match (kind, default_defines) {
        (shaderc::ShaderKind::Vertex, true) => {
            options.add_macro_definition("GEARS_VERTEX", None);
            options.add_macro_definition(
                "GEARS_IN(_location, _data)",
                Some("layout(location = _location) in _data;"),
            );
            options.add_macro_definition(
                "GEARS_INOUT(_location, _data)",
                Some("layout(location = _location) out _data;"),
            );
            options.add_macro_definition(
                "GEARS_VERT_UBO(_location, _data)",
                Some("layout(binding = _location) _data;"),
            );
        }
        (shaderc::ShaderKind::Fragment, true) => {
            options.add_macro_definition("GEARS_FRAGMENT", None);
            options.add_macro_definition(
                "GEARS_OUT(_location, _data)",
                Some("layout(location = _location) out _data;"),
            );
            options.add_macro_definition(
                "GEARS_INOUT(_location, _data)",
                Some("layout(location = _location) in _data;"),
            );
        }
        _ => (),
    };

    for (define, val) in defines.defines.iter() {
        options.add_macro_definition(define, val.as_ref().map_or(None, |s| Some(s.as_str())));
    }

    let result = if debug {
        compiler
            .preprocess(source, name, entry, Some(&options))
            .map_or_else(|err| Err(format!("{}", err)), |res| Err(res.as_text()))
    } else {
        compiler
            .compile_into_spirv(source, kind, name, entry, Some(&options))
            .or_else(|err| Err(format!("{}", err)))
    };

    result.or_else(|err| {
        let source_with_lines: String = source
            .lines()
            .enumerate()
            .map(|(i, line)| format!("{:-4}: {}\n", i + 1, line))
            .collect();

        Err(format!(
            "Error:\n{}\nSource:\n{}",
            err,
            source_with_lines.trim_end()
        ))
    })
}

/// # gears-pipeline main macro
///
/// ## usage
/// ### modules
/// First token is the module id.
/// It defines the shader module type.
/// - ```vertex: { /* module options */ }``` (with aliases ```vs``` and ```v```)
/// - ```fragment: { /* module options */ }``` (with aliases ```fs``` and ```f```)
/// ### module options
/// #### ```source: "..."```
/// Has aliases: ```src``` and ```s```
/// Raw text form GLSL source to be compiled.
/// Only one ```source``` or ```path``` can be given.
/// #### ```path: "..."```
/// Has alias: ```p```
/// Path to GLSL source to be compiled.
/// Fills ```include``` if not already given.
/// Only one ```source``` or ```path``` can be given.
/// #### ```include: "..."```
/// Has aliases: ```inc``` and ```i```
/// Path to be used with #include.
/// Overwrites ```include``` if already given.
/// #### ```define: ["NAME1" = "VALUE", "NAME2"]```
/// Has aliases: ```def``` and ```d```
/// Adds a list of macros.
/// #### ```no-autodefine```
/// Has aliases: ```na``` and ```n```
/// Disables gears-pipeline defines.
/// #### ```entry: "..."```
/// Has aliases: ```ep``` and ```e```
/// Specifies the entry point name.
/// #### ```debug```
/// Dumps glsl as a compile error
///
/// ## gears-pipeline defines
///
/// ### for vertex shaders:
///  - ```#define GEARS_VERTEX```
///  - ```#define GEARS_IN(_location, _data) layout(location = _location) in _data;```
///  - ```#define GEARS_INOUT(_location, _data) layout(location = _location) out _data;```
///  - ```#define GEARS_OUT(_location, _data) _data;```
///
/// ### for vertex shaders:
///  - ```#define GEARS_FRAGMENT```
///  - ```#define GEARS_IN(_location, _data) _data;```
///  - ```#define GEARS_INOUT(_location, _data) layout(location = _location) in _data;```
///  - ```#define GEARS_OUT(_location, _data) layout(location = _location) out _data;```
///
/// ## gears-pipeline default entry points
/// - vertex shader: ```vert```
/// - fragment shader: ```frag```
///
/// ### rust like attribute macros:
/// ```#[gears_bindgen]```
/// This expands a struct or uniform in the glsl source and generates rust bindings for it.
/// Arguments for it can be given after 'gears_bindgen' in parentheses.
/// Possible arguments:
///  - shader input: ```in```
///  - shader output: ```out```
///  - uniforms: ```unifom(binding = 0)``` (the binding can be any integer)
///
/// ```#[gears_gen]```
/// This is the same as ```#[gears_bindgen]``` but will not generate the rust bindings.
///
/// ### example
/// ```
/// mod pl {
///     gears_pipeline::pipeline! {
///         vs: {
///             path: "tests/test.glsl"
///             def: [ "FRAGMENT", "VALUE" = "2" ]
///         }
///         fs: {
///             source: "#version 440\n#include \"include.glsl\""
///             include: "tests/"
///         }
///     }
/// }
///
/// // check SPIRV generation
/// assert_eq!(1248, pl::VERT_SPIRV.len(), "Vert spirv not what expected");
/// assert_eq!(252, pl::FRAG_SPIRV.len(), "Frag spirv not what expected");
///
/// // check UBO struct generation
/// pl::UBO { time: 0f32 };
/// ```
#[proc_macro]
pub fn pipeline(input: TokenStream) -> TokenStream {
    match Pipeline::new(parse_macro_input!(input as PipelineInput)) {
        Err(err) => err.to_compile_error().into(),
        Ok(pipeline) => {
            let mut tokens = proc_macro2::TokenStream::new();
            pipeline.to_tokens(&mut tokens);

            tokens.into()
        }
    }
}
