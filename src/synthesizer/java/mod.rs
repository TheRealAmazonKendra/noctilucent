use super::Synthesizer;
use crate::code::{CodeBuffer, IndentOptions};
use crate::ir::conditions::ConditionIr;
use crate::ir::importer::ImportInstruction;
use crate::ir::reference::{Origin, PseudoParameter, Reference};
use crate::ir::resources::{ResourceInstruction, ResourceIr};
use crate::ir::CloudformationProgramIr;
use crate::parser::lookup_table::MappingInnerValue;
use crate::specification::Structure;
use std::borrow::Cow;
use std::io;
use std::rc::Rc;
use voca_rs::case::{camel_case, pascal_case};

const INDENT: Cow<'static, str> = Cow::Borrowed("    ");
const DOUBLE_INDENT: Cow<'static, str> = Cow::Borrowed("        ");

macro_rules! fill {
    ($code:ident; $leading:expr; $($lines:expr),* ; $trailing:expr) => {
        {
            let _class = $code.indent_with_options(IndentOptions {
                indent: INDENT,
                leading: Some($leading.into()),
                trailing: Some($trailing.into()),
                trailing_newline: true,
            });

            $(_class.line(format!($lines));)*
        }
    };
}

pub struct Java {
    package_name: String,
}

impl Java {
    pub fn new(package_name: impl Into<String>) -> Self {
        Self {
            package_name: package_name.into(),
        }
    }

    //noinspection ALL
    fn write_header(&self, code: &CodeBuffer) {
        code.line(format!("package {};", self.package_name));

        // base imports
        code.newline();
        code.line("import software.constructs.Construct;");
        code.newline();
        code.line("import java.util.*;");
        code.line("import software.amazon.awscdk.*;");
        code.line("import software.amazon.awscdk.CfnMapping;");
        code.line("import software.amazon.awscdk.CfnTag;");
        code.line("import software.amazon.awscdk.Stack;");
        code.line("import software.amazon.awscdk.StackProps;");
    }

    fn emit_mappings(mapping: &MappingInnerValue, output: &CodeBuffer) {
        match mapping {
            MappingInnerValue::Number(num) => output.text(format!("{num}")),
            MappingInnerValue::Bool(bool) => output.text(if *bool { "true" } else { "false" }),
            MappingInnerValue::String(str) => output.text(format!("{str:?}")),
            MappingInnerValue::List(items) => {
                output.text(format!("Arrays.asList(\"{}\")", items.join("\", \"")))
            }
            MappingInnerValue::Float(num) => output.text(format!("{num}")),
        };
    }

    fn write_mappings(ir: &CloudformationProgramIr, map: &CodeBuffer) {
        if ir.mappings.is_empty() {
            return;
        }
        map.line("// Mappings");
        for mapping in &ir.mappings {
            let name = camel_case(&mapping.name);
            map.line(format!(
                "final CfnMapping {name} = new CfnMapping(this, \"{name}\");"
            ));
            for (key1, inner_mapping) in &mapping.map {
                for (key2, value) in inner_mapping {
                    map.text(format!("{name}.setValue(\"{key1}\", \"{key2}\", "));
                    Self::emit_mappings(value, map);
                    map.text(");\n");
                }
            }
            map.newline();
        }
    }

    fn emit_props(ir: &CloudformationProgramIr) -> Vec<JavaConstructorParameter> {
        let mut v = Vec::new();
        for input in &ir.constructor.inputs {
            let java_type = match input.constructor_type.as_str() {
                "List<Number>" => "List<Number>",
                t if t.contains("List") => "String[]",
                _ => "String",
            };

            v.push(JavaConstructorParameter {
                name: input.name.clone(),
                description: input.description.clone(),
                java_type: java_type.into(),
                constructor_type: input.constructor_type.clone(),
                default_value: input.default_value.clone(),
            });
        }
        v
    }

    fn write_stack_definitions(
        props: &Vec<JavaConstructorParameter>,
        writer: &CodeBuffer,
        stack_name: &str,
    ) -> Rc<CodeBuffer> {
        fill!(writer;
            format!("public {}(final Construct scope, final String id) {{", stack_name);
            "super(scope, id, null);";
            "}" );

        writer.newline();
        let definitions = writer.indent_with_options(IndentOptions {
                indent: INDENT,
                leading: Some(
                    format!(
                        "public {stack_name}(final Construct scope, final String id, final StackProps props) {{",
                    )
                    .into(),
                ),
                trailing: Some("}".into()),
                trailing_newline: true,
            });

        if props.is_empty() {
            definitions.line("super(scope, id, props);");
            definitions
        } else {
            definitions.line(format!(
                "this(scope, id, props{});",
                ", null".repeat(props.len())
            ));

            writer.newline();
            let definitions_with_props = writer.indent_with_options(IndentOptions {
                indent: INDENT,
                leading: Some(format!("public {stack_name}(final Construct scope, final String id, final StackProps props,").into()),
                trailing: Some("}".into()),
                trailing_newline: true,
            });
            let mut prop = props.iter().peekable();
            while let Some(p) = prop.next() {
                if prop.peek().is_none() {
                    definitions_with_props
                        .indent(INDENT)
                        .line(format!("{} {}) {{", p.java_type, p.name));
                } else {
                    definitions_with_props
                        .indent(INDENT)
                        .line(format!("{} {},", p.java_type, p.name));
                }
            }
            definitions_with_props.line("super(scope, id, props);");
            definitions_with_props.newline();
            definitions_with_props
        }
    }

    fn write_props(props: &Vec<JavaConstructorParameter>, writer: &CodeBuffer) {
        for prop in props {
            match &prop.default_value {
                None => writer.newline(),
                Some(v) if prop.constructor_type.contains("AWS::") => {
                    let value_as = match &prop.constructor_type {
                        t if t.contains("List") => "getValueAsList",
                        _ => "getValueAsString",
                    };
                    let prop_options = writer.indent_with_options(IndentOptions {
                        indent: DOUBLE_INDENT,
                        leading: Some(
                            format!(
                                "{} = Optional.ofNullable({}).isPresent()",
                                prop.name, prop.name
                            )
                            .into(),
                        ),
                        trailing: None,
                        trailing_newline: true,
                    });
                    prop_options.line(format!("? {}", prop.name));
                    let prop_details = prop_options.indent_with_options(IndentOptions {
                        indent: DOUBLE_INDENT,
                        leading: Some(
                            format!(
                                ": CfnParameter.Builder.create(this, \"{}\")",
                                pascal_case(&prop.name)
                            )
                            .into(),
                        ),
                        trailing: None,
                        trailing_newline: false,
                    });
                    prop_details.line(format!(".type(\"{}\")", prop.constructor_type));
                    prop_details.line(format!(".defaultValue(\"{}\")", v));
                    prop_details.line(".build()");
                    prop_details.line(format!(".{}();", value_as));
                }
                Some(v) => writer.line(format!(
                    "{} = Optional.ofNullable({}).isPresent() ? {}\n{DOUBLE_INDENT}: \"{}\";",
                    prop.name, prop.name, prop.name, v
                )),
            }
        }
    }

    fn write_resource(resource: &ResourceInstruction, writer: &Rc<CodeBuffer>) -> bool {
        let class = resource.resource_type.type_name();
        let res_name = &resource.name;

        if let Some(cond) = &resource.condition {
            writer.line(format!("Optional<Cfn{class}> {} = {} ? Optional.of(Cfn{class}.Builder.create(this, \"{res_name}\")", name(res_name), camel_case(cond)));
            let properties = writer.indent(DOUBLE_INDENT);
            for (name, prop) in &resource.properties {
                properties.text(format!(".{}(", camel_case(name)));
                emit_java(prop.clone(), &properties, Some(class));
                properties.text(")\n");
            }
            properties.line(".build()) : Optional.empty();");
            true
        } else {
            writer.line(format!(
                "Cfn{class} {} = Cfn{class}.Builder.create(this, \"{res_name}\")",
                name(res_name)
            ));
            let properties = writer.indent(DOUBLE_INDENT);
            for (name, prop) in &resource.properties {
                properties.text(format!(".{}(", camel_case(name)));
                emit_java(prop.clone(), &properties, Some(class));
                properties.text(")\n");
            }
            properties.line(".build();");
            false
        }
    }

    fn write_resources(ir: &CloudformationProgramIr, writer: &Rc<CodeBuffer>) {
        for resource in &ir.resources {
            let maybe_undefined = Self::write_resource(resource, writer);
            writer.newline();
            Self::write_resource_attributes(resource, writer, maybe_undefined);
        }
    }

    fn write_resource_attributes(
        resource: &ResourceInstruction,
        writer: &Rc<CodeBuffer>,
        maybe_undefined: bool,
    ) {
        let res_name = if maybe_undefined {
            format!(
                "{}.ifPresent(_{} -> _{}",
                camel_case(&resource.name),
                camel_case(&resource.name),
                camel_case(&resource.name)
            )
        } else {
            camel_case(&resource.name)
        };
        let trailer = if maybe_undefined { ");\n" } else { ";\n" };
        let mut extra_line = false;

        if let Some(metadata) = &resource.metadata {
            match metadata {
                ResourceIr::Object(_, entries) => {
                    for (name, value) in entries {
                        writer.text(format!("{res_name}.addMetadata(\"{name}\", "));
                        emit_java(value.clone(), writer, None);
                        writer.text(format!("){trailer}"));
                    }
                }
                unsupported => {
                    writer.line(format!("/* {unsupported:?} */"));
                }
            }
            extra_line = true;
        }

        for dependency in &resource.dependencies {
            writer.text(format!(
                "{res_name}.addDependency({}){}",
                dependency.to_lowercase(),
                trailer
            ));
            extra_line = true;
        }

        if let Some(deletion_policy) = &resource.deletion_policy {
            writer.text(format!(
                "{res_name}.applyRemovalPolicy(RemovalPolicy.{deletion_policy}){}",
                trailer
            ));
            extra_line = true;
        }

        if let Some(update_policy) = &resource.update_policy {
            writer.text(format!("{res_name}.getCfnOptions().setUpdatePolicy("));
            emit_java(update_policy.clone(), writer, None);
            writer.text(format!("){trailer}"));
            extra_line = true;
        }
        if extra_line {
            writer.newline();
        }
    }

    fn write_conditions(ir: &CloudformationProgramIr, writer: &Rc<CodeBuffer>) {
        for condition in &ir.conditions {
            let name = &*condition.name;
            let val = &condition.value;
            writer.line(format!(
                "Boolean {} = {};",
                camel_case(name),
                emit_conditions(val.clone())
            ));
        }
        writer.newline();
    }

    fn match_field_type(condition: Option<String>) -> String {
        String::from(match condition {
            None => "Object",
            Some(_) => "Optional<Object>",
        })
    }

    fn write_output_fields(ir: &CloudformationProgramIr, writer: &Rc<CodeBuffer>) {
        for output in &ir.outputs {
            writer.line(format!(
                "private {} {};\n",
                Self::match_field_type(output.condition.clone()),
                camel_case(&output.name)
            ))
        }

        for output in &ir.outputs {
            let indented = writer.indent_with_options(IndentOptions {
                indent: INDENT,
                leading: Some(
                    format!(
                        "public {} get{}() {{",
                        Self::match_field_type(output.condition.clone()),
                        pascal_case(&output.name)
                    )
                    .into(),
                ),
                trailing: Some("}\n".into()),
                trailing_newline: true,
            });
            indented.line(format!("return this.{};", camel_case(&output.name)));
        }
    }

    fn write_outputs(ir: &CloudformationProgramIr, writer: &Rc<CodeBuffer>) {
        for output in &ir.outputs {
            let var_name = camel_case(&output.name);
            let output_writer = match &output.condition {
                None => {
                    writer.text(format!("this.{var_name} = "));
                    emit_java(output.value.clone(), writer, None);
                    writer.text(";\n");
                    let output_writer = writer.indent_with_options(IndentOptions {
                        indent: DOUBLE_INDENT,
                        leading: Some(
                            format!("CfnOutput.Builder.create(this, \"{}\")", &output.name).into(),
                        ),
                        trailing: Some(format!("{DOUBLE_INDENT}.build();").into()),
                        trailing_newline: true,
                    });
                    output_writer.line(format!(".value(this.{var_name}.toString())"));
                    output_writer
                }
                Some(cond) => {
                    writer.text(format!(
                        "this.{} = {} ? ",
                        camel_case(&output.name),
                        camel_case(cond)
                    ));
                    emit_java(output.value.clone(), writer, None);
                    writer.text(" : Optional.empty();\n");
                    let output_writer = writer.indent_with_options(IndentOptions {
                        indent: DOUBLE_INDENT,
                        leading: Some(
                            format!(
                                "this.{var_name}.ifPresent(_{var_name} -> CfnOutput.Builder.create(this, \"{}\")",
                                &output.name
                            )
                            .into(),
                        ),
                        trailing: Some(format!("{DOUBLE_INDENT}.build());").into()),
                        trailing_newline: true,
                    });
                    output_writer.line(format!(".value(_{var_name}.toString())"));
                    output_writer
                }
            };
            if output.description.is_some() {
                output_writer.line(format!(
                    ".description(\"{}\")",
                    output.description.clone().unwrap()
                ))
            }
            if output.export.is_some() {
                output_writer.text(".exportName(");
                emit_java(output.export.clone().unwrap(), &output_writer, None);
                output_writer.text(")\n");
            }
            writer.newline();
        }
    }
}

impl Default for Java {
    fn default() -> Self {
        Self::new("com.myorg")
    }
}

impl Synthesizer for Java {
    fn synthesize(
        &self,
        ir: CloudformationProgramIr,
        into: &mut dyn io::Write,
        stack_name: &str,
    ) -> io::Result<()> {
        let code = CodeBuffer::default();

        self.write_header(&code);

        for import in &ir.imports {
            code.line(import.to_java_import());
        }
        code.newline();

        let class = code.indent_with_options(IndentOptions {
            indent: INDENT,
            leading: Some(format!("class {} extends Stack {{", stack_name).into()),
            trailing: Some("}".into()),
            trailing_newline: true,
        });

        let props = Self::emit_props(&ir);
        Self::write_output_fields(&ir, &class);

        let definitions = Self::write_stack_definitions(&props, &class, stack_name);
        Self::write_props(&props, &definitions);

        Self::write_mappings(&ir, &definitions);
        Self::write_conditions(&ir, &definitions);
        Self::write_resources(&ir, &definitions);
        Self::write_outputs(&ir, &definitions);

        code.write(into)
    }
}

impl ImportInstruction {
    fn to_java_import(&self) -> String {
        let mut parts: Vec<Cow<str>> = vec![match self.path[0].as_str() {
            "aws-cdk-lib" => "software.amazon.awscdk.services".into(),
            other => other.into(),
        }];
        parts.extend(self.path[1..].iter().map(|item| {
            item.chars()
                .filter(|ch| ch.is_alphanumeric())
                .collect::<String>()
                .into()
        }));

        let module = parts
            .iter()
            .take(parts.len() - 1)
            .map(|part| part.to_string())
            .collect::<Vec<_>>()
            .join(".");
        if !module.is_empty() {
            format!(
                "import {module}.{name}.*;",
                module = module,
                name = self.name,
            )
        } else {
            "".to_string()
        }
    }
}

fn emit_conditions(condition: ConditionIr) -> String {
    match condition {
        ConditionIr::Ref(reference) => emit_reference(reference),
        ConditionIr::Str(str) => format!("{str:?}"),
        ConditionIr::Condition(x) => camel_case(&x),
        ConditionIr::And(list) => {
            let and = get_condition(list, " && ");
            format!("({and})")
        }
        ConditionIr::Or(list) => {
            let or = get_condition(list, " || ");
            format!("({or})")
        }
        ConditionIr::Not(cond) => {
            if cond.is_simple() {
                format!("!{}", emit_conditions(*cond))
            } else {
                format!("!({})", emit_conditions(*cond))
            }
        }
        ConditionIr::Equals(lhs, rhs) => {
            format!(
                "{}.equals({})",
                emit_conditions(*lhs),
                emit_conditions(*rhs)
            )
        }
        ConditionIr::Map(_, tlk, slk) => {
            format!(
                "Fn.map({}, {})",
                emit_conditions(*tlk),
                emit_conditions(*slk)
            )
        }
        ConditionIr::Split(sep, l1) => {
            let str = emit_conditions(l1.as_ref().clone());
            format!("Arrays.asList({str}.split(\"{sep}\"))")
        }
        ConditionIr::Select(index, str) => {
            format!("Fn.select({index:?}, {})", emit_conditions(*str))
        }
    }
}

fn emit_reference(reference: Reference) -> String {
    let origin = reference.origin;
    let name = reference.name;
    match origin {
        Origin::LogicalId { conditional } => {
            if conditional {
                format!(
                    "Optional.of({}.isPresent() ? {}.get().getRef()\n{DOUBLE_INDENT}: Optional.empty())",
                    camel_case(&name),
                    camel_case(&name)
                )
            } else {
                format!("{}.getRef()", camel_case(&name))
            }
        }
        Origin::GetAttribute {
            conditional,
            attribute,
        } => {
            if conditional {
                format!(
                    "Optional.of({}.isPresent() ? {}.get().getAttr{}()\n{DOUBLE_INDENT}: Optional.empty())",
                    camel_case(&name),
                    camel_case(&name),
                    pascal_case(&attribute)
                )
            } else {
                format!("{}.getAttr{}()", camel_case(&name), pascal_case(&attribute))
            }
        }
        Origin::PseudoParameter(param) => get_pseudo_param(param),
        Origin::Parameter => camel_case(&name),
        Origin::Condition => name,
    }
}

fn get_pseudo_param(param: PseudoParameter) -> String {
    match param {
        PseudoParameter::Partition => "this.getPartition()",
        PseudoParameter::Region => "this.getRegion()",
        PseudoParameter::StackId => "this.getStackId()",
        PseudoParameter::StackName => "this.getStackName()",
        PseudoParameter::URLSuffix => "this.getUrlSuffix()",
        PseudoParameter::AccountId => "this.getAccount()",
        PseudoParameter::NotificationArns => "this.getNotificationArns()",
    }
    .into()
}

fn get_condition(list: Vec<ConditionIr>, sep: &str) -> String {
    list.into_iter()
        .map(emit_conditions)
        .collect::<Vec<_>>()
        .join(sep)
}

fn emit_tag_value(this: ResourceIr, output: &CodeBuffer, class: Option<&str>) {
    match this {
        ResourceIr::Bool(bool) => output.text(format!("String.valueOf({bool})")),
        ResourceIr::Double(number) => output.text(format!("String.valueOf({number})")),
        ResourceIr::Number(number) => output.text(format!("String.valueOf({number})")),
        other => emit_java(other, output, class),
    }
}

fn emit_java(this: ResourceIr, output: &CodeBuffer, class: Option<&str>) {
    match this {
        // Literal values
        ResourceIr::Null => output.text("null"),
        ResourceIr::Bool(bool) => output.text(bool.to_string()),
        ResourceIr::Double(number) => output.text(format!("{number}")),
        ResourceIr::Number(number) => output.text(format!("{number}")),
        ResourceIr::String(text) => output.text(format!("\"{text}\"")),

        // Collection values
        ResourceIr::Array(_, array) => {
            let arr_writer = output.indent_with_options(IndentOptions {
                indent: DOUBLE_INDENT,
                leading: Some("Arrays.asList(".into()),
                trailing: None,
                trailing_newline: false,
            });
            let mut arr = array.iter().peekable();
            while let Some(resource) = arr.next() {
                if arr.peek().is_none() {
                    emit_java(resource.clone(), &arr_writer, class);
                    arr_writer.text(")");
                } else {
                    emit_java(resource.clone(), &arr_writer, class);
                    arr_writer.text(",\n");
                }
            }
        }
        ResourceIr::Object(structure, entries) => match structure {
            Structure::Composite(property) => match property {
                "Tag" => {
                    let obj = output.indent_with_options(IndentOptions {
                        indent: DOUBLE_INDENT,
                        leading: Some("CfnTag.builder()".into()),
                        trailing: Some(format!("{DOUBLE_INDENT}.build()").into()),
                        trailing_newline: false,
                    });
                    for (key, value) in &entries {
                        if key.eq_ignore_ascii_case("Key") {
                            obj.text(".key(");
                            emit_java(value.clone(), &obj, class);
                            obj.text(")\n");
                        }
                        if key.eq_ignore_ascii_case("Value") {
                            obj.text(".value(");
                            emit_tag_value(value.clone(), &obj, class);
                            obj.text(")\n")
                        }
                    }
                }
                _ => {
                    let obj = output.indent_with_options(IndentOptions {
                        indent: DOUBLE_INDENT,
                        leading: Some(
                            format!("Cfn{}.{property}Property.builder()", class.unwrap()).into(),
                        ),
                        trailing: Some(format!("{DOUBLE_INDENT}.build()").into()),
                        trailing_newline: false,
                    });
                    for (key, value) in &entries {
                        obj.text(format!(".{}(", camel_case(key)));
                        emit_java(value.clone(), &obj, class);
                        obj.text(")\n");
                    }
                }
            },
            Structure::Simple(_) => {
                output.text("Map.of(");
                let mut map = entries.iter().peekable();
                while let Some((key, value)) = map.next() {
                    output.text(format!("\"{key}\", "));
                    emit_java(value.clone(), output, class);
                    if map.peek().is_some() {
                        output.text(",\n");
                    } else {
                        output.text(")");
                    }
                }
            }
        },

        // Intrinsics
        ResourceIr::Base64(base64) => match base64.as_ref() {
            ResourceIr::String(b64) => {
                output.text(format!(
                    "new String(Base64.getDecoder().decode(\"{}\"))",
                    b64.escape_debug()
                ));
            }
            other => {
                output.text("Fn.base64(");
                emit_java(other.clone(), output, class);
                output.text(")");
            }
        },
        ResourceIr::Cidr(cidr_block, count, mask) => {
            output.text("Fn.cidr(");
            emit_java(*cidr_block, output, class);
            output.text(", ");
            emit_java(*count, output, class);
            output.text(", ");
            match mask.as_ref() {
                ResourceIr::Number(mask) => {
                    output.text(format!("\"{mask}\""));
                }
                ResourceIr::String(mask) => {
                    output.text(format!("{mask:?}"));
                }
                mask => output.text(format!("String.valueOf({mask:?})")),
            }
            output.text(")");
        }
        ResourceIr::GetAZs(region) => {
            output.text("Fn.getAzs(");
            emit_java(*region, output, None);
            output.text(")");
        }
        ResourceIr::If(cond_name, if_true, if_false) => {
            output.text(format!("{} ? ", camel_case(&cond_name)));
            emit_java(*if_true, output, class);
            output.text(format!("\n{DOUBLE_INDENT}: "));
            emit_java(*if_false, output, class);
        }
        ResourceIr::ImportValue(text) => output.text(format!("Fn.importValue(\"{text}\")")),
        ResourceIr::Join(sep, list) => {
            let items = output.indent_with_options(IndentOptions {
                indent: DOUBLE_INDENT,
                leading: Some(format!("String.join(\"{sep}\",").into()),
                trailing: Some(")".into()),
                trailing_newline: false,
            });
            let mut l = list.iter().peekable();
            while let Some(item) = l.next() {
                emit_java(item.clone(), &items, class);
                if l.peek().is_some() {
                    items.text(",\n");
                }
            }
        }
        ResourceIr::Map(name, tlk, slk) => {
            output.text(format!("{}.findInMap(", camel_case(&name)));
            emit_java(*tlk, output, class);
            output.text(", ");
            emit_java(*slk, output, class);
            output.text(")");
        }
        ResourceIr::Select(idx, list) => match list.as_ref() {
            ResourceIr::Array(_, array) => {
                if idx <= array.len() {
                    emit_java(array[idx].clone(), output, class)
                } else {
                    output.text("null");
                }
            }
            list => {
                output.text(format!("Fn.select({idx}, "));
                emit_java(list.clone(), output, class);
                output.text(")");
            }
        },
        ResourceIr::Split(separator, resource) => match resource.as_ref() {
            ResourceIr::String(str) => {
                output.text(format!("{str}.split(\"{separator}\")"));
            }
            other => {
                output.text(format!("Fn.split({separator}, "));
                emit_java(other.clone(), output, class);
                output.text(")");
            }
        },
        ResourceIr::Sub(parts) => {
            let mut part = parts.iter().peekable();
            while let Some(p) = part.next() {
                match p {
                    ResourceIr::String(lit) => output.text(format!("\"{}\"", lit.clone())),
                    other => emit_java(other.clone(), output, class),
                }
                if part.peek().is_some() {
                    output.text(" + ");
                }
            }
        }

        ResourceIr::Ref(reference) => output.text(emit_reference(reference)),
    }
}

fn name(key: &str) -> String {
    camel_case(key)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

trait JavaCodeBuffer {
    fn java_doc(&self) -> Rc<CodeBuffer>;
}

impl JavaCodeBuffer for CodeBuffer {
    #[inline]
    fn java_doc(&self) -> Rc<CodeBuffer> {
        self.indent_with_options(IndentOptions {
            indent: " * ".into(),
            leading: Some("/**".into()),
            trailing: Some(" */".into()),
            trailing_newline: true,
        })
    }
}

pub struct JavaConstructorParameter {
    pub name: String,
    pub description: Option<String>,
    pub constructor_type: String,
    pub java_type: String,
    pub default_value: Option<String>,
}

pub struct JavaResourceInstruction {}

#[cfg(test)]
mod tests {}
