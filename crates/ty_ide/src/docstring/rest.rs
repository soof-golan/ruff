use std::borrow::Cow;
use std::collections::HashMap;

use ruff_python_trivia::leading_indentation;
use ruff_source_file::UniversalNewlines;

use super::markdown;

/// Extracts parameter documentation from reST field lists in a docstring.
pub(super) struct Parser<'a> {
    docstring: &'a str,
}

impl<'a> Parser<'a> {
    pub(super) const fn new(docstring: &'a str) -> Self {
        Self { docstring }
    }

    pub(super) fn parameter_documentation(&self) -> Vec<ParameterDocumentation> {
        FieldList::parse_all(self.docstring)
            .into_iter()
            .flat_map(|field_list| field_list.fields)
            .filter_map(|field| match field {
                Field::Parameter {
                    name, description, ..
                } if !description.is_empty() => Some(ParameterDocumentation { name, description }),
                _ => None,
            })
            .collect()
    }
}

/// Renders supported top-level reST field lists as Markdown sections.
pub(super) struct Formatter<'a> {
    docstring: &'a str,
}

impl<'a> Formatter<'a> {
    pub(super) const fn new(docstring: &'a str) -> Self {
        Self { docstring }
    }

    /// Returns the original docstring when no supported field list is rendered.
    pub(super) fn render_field_lists(&self) -> Cow<'a, str> {
        let docstring = self.docstring;
        let lines = docstring.lines().collect::<Vec<_>>();
        let mut rendered: Option<Vec<String>> = None;
        let mut code_examples = CodeExampleTracker::default();
        let mut index = 0;

        while index < lines.len() {
            let line = lines[index];
            // Field-like text inside code examples stays literal.
            if code_examples.contains_current_line(line) {
                if let Some(rendered) = &mut rendered {
                    rendered.push(line.to_string());
                }
                index += 1;
            } else if FieldStart::indentation(line) == 0 && FieldStart::parse(line).is_some() {
                let (field_list, next_index) = FieldList::parse(&lines, index);
                if let Some(field_list_markdown) = Self::render_field_list(&field_list) {
                    // Keep the borrowed docstring until a field list actually changes.
                    let rendered = rendered.get_or_insert_with(|| {
                        let mut rendered = Vec::with_capacity(lines.len());
                        rendered.extend(lines[..index].iter().map(|line| (*line).to_string()));
                        rendered
                    });
                    rendered.push(field_list_markdown);
                } else if let Some(rendered) = &mut rendered {
                    rendered.extend(
                        lines[index..next_index]
                            .iter()
                            .map(|line| (*line).to_string()),
                    );
                }
                index = next_index;
            } else {
                if let Some(rendered) = &mut rendered {
                    rendered.push(line.to_string());
                }
                code_examples.observe_plaintext_line(line);
                index += 1;
            }
        }

        if let Some(rendered) = rendered {
            Cow::Owned(rendered.join("\n"))
        } else {
            Cow::Borrowed(docstring)
        }
    }

    fn render_field_list(field_list: &FieldList) -> Option<String> {
        let parameters = field_list
            .fields
            .iter()
            .filter_map(|field| match field {
                Field::Parameter {
                    name,
                    ty,
                    description,
                } => Some((name.as_str(), ty.as_deref(), description.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();

        let returns = field_list
            .fields
            .iter()
            .filter_map(|field| match field {
                Field::Returns { name, description } => {
                    Some((name.as_deref(), description.as_str()))
                }
                _ => None,
            })
            .collect::<Vec<_>>();

        let return_type = field_list.fields.iter().find_map(|field| match field {
            Field::ReturnType { ty } => Some(ty.as_str()),
            _ => None,
        });

        let raises = field_list
            .fields
            .iter()
            .filter_map(|field| match field {
                Field::Raises {
                    exception,
                    description,
                } => Some((exception.as_deref(), description.as_str())),
                _ => None,
            })
            .collect::<Vec<_>>();

        let has_rendered_fields = !parameters.is_empty()
            || !returns.is_empty()
            || return_type.is_some()
            || !raises.is_empty();

        if !has_rendered_fields {
            return None;
        }

        let mut output = String::new();
        if !parameters.is_empty() {
            Self::start_markdown_section(&mut output, "Parameters");
            let parameter_types = Self::parameter_types(field_list);
            for (name, ty, description) in parameters {
                let ty = ty.or_else(|| parameter_types.get(name).copied());
                output.push_str(&Self::render_field_entry(Some(name), ty, description));
                output.push('\n');
            }
            output.pop();
        }

        if !returns.is_empty() || return_type.is_some() {
            Self::start_markdown_section(&mut output, "Returns");
            if returns.is_empty() {
                output.push_str(&Self::render_field_entry(None, return_type, ""));
            } else {
                for (name, description) in returns {
                    output.push_str(&Self::render_field_entry(name, return_type, description));
                    output.push('\n');
                }
                output.pop();
            }
        }

        if !raises.is_empty() {
            Self::start_markdown_section(&mut output, "Raises");
            for (exception, description) in raises {
                output.push_str(&Self::render_field_entry(exception, None, description));
                output.push('\n');
            }
            output.pop();
        }

        let raw_fields = Self::raw_fields_to_preserve(field_list);
        if !raw_fields.is_empty() {
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str(&raw_fields.join("\n"));
        }

        Some(output)
    }

    fn start_markdown_section(output: &mut String, heading: &str) {
        if !output.is_empty() {
            output.push_str("\n\n");
        }
        output.push_str("## ");
        output.push_str(heading);
        output.push('\n');
    }

    fn parameter_types(field_list: &FieldList) -> HashMap<&str, &str> {
        let mut parameter_types = HashMap::new();
        for field in &field_list.fields {
            if let Field::ParameterType { name, ty, .. } = field
                && !ty.is_empty()
            {
                parameter_types.entry(name.as_str()).or_insert(ty.as_str());
            }
        }
        parameter_types
    }

    fn raw_fields_to_preserve(field_list: &FieldList) -> Vec<&str> {
        let parameter_names = field_list
            .fields
            .iter()
            .filter_map(|field| match field {
                Field::Parameter { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        field_list
            .fields
            .iter()
            .filter_map(|field| match field {
                Field::ParameterType { name, raw, .. }
                    if !parameter_names.contains(&name.as_str()) =>
                {
                    Some(raw.as_str())
                }
                Field::Unsupported { raw } => Some(raw.as_str()),
                _ => None,
            })
            .collect()
    }

    fn render_field_entry(name: Option<&str>, ty: Option<&str>, description: &str) -> String {
        let mut entry = String::new();
        if let Some(name) = name {
            entry.push_str(&Self::markdown_code_span(name));
        }

        if let Some(ty) = ty
            && !ty.is_empty()
        {
            if !entry.is_empty() {
                entry.push(' ');
            }
            entry.push('(');
            entry.push_str(&Self::markdown_code_span(ty));
            entry.push(')');
        }

        if !description.is_empty() {
            let starts_with_markdown_block =
                Self::description_starts_with_markdown_block(description);
            if !entry.is_empty() {
                if starts_with_markdown_block {
                    entry.push_str(":\n");
                } else {
                    entry.push_str(": ");
                }
            }
            if starts_with_markdown_block {
                entry.push_str(description);
            } else {
                entry.push_str(&description.replace('\n', "\n    "));
            }
        }

        entry
    }

    fn description_starts_with_markdown_block(description: &str) -> bool {
        description.lines().next().is_some_and(|first_line| {
            let first_line = first_line.trim_start_matches(' ');
            markdown::fence_start(first_line).is_some() || first_line.starts_with(">>>")
        })
    }

    fn markdown_code_span(text: &str) -> String {
        let longest_backtick_run = text
            .split(|char| char != '`')
            .map(str::len)
            .max()
            .unwrap_or(0);
        let delimiter = "`".repeat(longest_backtick_run + 1);
        if text.starts_with('`') || text.ends_with('`') {
            format!("{delimiter} {text} {delimiter}")
        } else {
            format!("{delimiter}{text}{delimiter}")
        }
    }
}

/// Parameter documentation extracted from a reST field list.
pub(super) struct ParameterDocumentation {
    pub(super) name: String,
    pub(super) description: String,
}

/// A contiguous block of adjacent reST field entries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldList {
    fields: Vec<Field>,
}

impl FieldList {
    fn parse_all(docstring: &str) -> Vec<Self> {
        let lines: Vec<&str> = docstring
            .universal_newlines()
            .map(|line| line.as_str())
            .collect();
        let mut field_lists = Vec::new();
        let mut code_examples = CodeExampleTracker::default();
        let mut index = 0;

        while index < lines.len() {
            let line = lines[index];
            if code_examples.contains_current_line(line) {
                index += 1;
            } else if FieldStart::parse(line).is_some() {
                let (field_list, next_index) = Self::parse(&lines, index);
                if !field_list.fields.is_empty() {
                    field_lists.push(field_list);
                }
                index = next_index;
            } else {
                code_examples.observe_plaintext_line(line);
                index += 1;
            }
        }

        field_lists
    }

    fn parse(lines: &[&str], start: usize) -> (Self, usize) {
        debug_assert!(start < lines.len());
        debug_assert!(FieldStart::parse(lines[start]).is_some());

        let field_list_indent = FieldStart::indentation(lines[start]);
        let mut fields = Vec::new();
        let mut current: Option<FieldBuilder> = None;
        let mut index = start;

        while let Some(line) = lines.get(index) {
            if let Some(field) = Self::field_start_at_indent(line, field_list_indent) {
                if let Some(field) = current.take().map(FieldBuilder::finish) {
                    fields.push(field);
                }
                current = Some(FieldBuilder::new(field));
                index += 1;
                continue;
            }

            let Some(field) = &mut current else {
                break;
            };

            let trimmed = line.trim();
            if trimmed.is_empty() {
                // Blank lines belong to the field only when another field or
                // an indented continuation follows.
                let mut next_non_blank = index + 1;
                while let Some(next_line) = lines.get(next_non_blank)
                    && next_line.trim().is_empty()
                {
                    next_non_blank += 1;
                }

                if let Some(next_line) = lines.get(next_non_blank)
                    && (Self::field_start_at_indent(next_line, field.indent).is_some()
                        || FieldStart::indentation(next_line) > field.indent)
                {
                    field.raw_lines.push(line);
                    index += 1;
                    continue;
                }

                break;
            }

            let indent = FieldStart::indentation(line);
            if indent > field.indent {
                field.raw_lines.push(line);
                index += 1;
            } else {
                break;
            }
        }

        if let Some(field) = current.map(FieldBuilder::finish) {
            fields.push(field);
        }

        debug_assert!(index > start);

        (Self { fields }, index)
    }

    fn field_start_at_indent(line: &str, indent: usize) -> Option<FieldStart<'_>> {
        if FieldStart::indentation(line) == indent {
            FieldStart::parse(line)
        } else {
            None
        }
    }
}

/// A parsed field with its body collected.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Field {
    /// `:param name:` with an optional inline type.
    Parameter {
        name: String,
        ty: Option<String>,
        description: String,
    },
    /// `:type name:`.
    ParameterType {
        name: String,
        ty: String,
        raw: String,
    },
    /// `:return:` or `:returns:`, optionally named.
    Returns {
        name: Option<String>,
        description: String,
    },
    /// `:rtype:`.
    ReturnType { ty: String },
    /// `:raises exception:`.
    Raises {
        exception: Option<String>,
        description: String,
    },
    /// A field that is preserved verbatim.
    Unsupported { raw: String },
}

/// Tracks code examples so field-like examples stay literal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct CodeExampleTracker {
    markdown_fence: Option<String>,
    rest_literal_block: LiteralBlockTracker,
    in_doctest: bool,
}

impl CodeExampleTracker {
    fn contains_current_line(&mut self, line: &str) -> bool {
        if let Some(fence) = &self.markdown_fence {
            if markdown::closes_fence(line, fence) {
                self.markdown_fence = None;
            }
            return true;
        }

        if self.rest_literal_block.contains_current_line(line) {
            return true;
        }

        if self.in_doctest {
            if line.trim_start_matches(' ').is_empty() {
                self.in_doctest = false;
            }
            return true;
        }

        if line.trim_start_matches(' ').starts_with(">>>") {
            self.in_doctest = true;
            return true;
        }

        if let Some(fence) = markdown::fence_start(line) {
            self.markdown_fence = Some(fence.to_string());
            return true;
        }

        false
    }

    fn observe_plaintext_line(&mut self, line: &str) {
        self.rest_literal_block.observe_plaintext_line(line);
    }
}

/// Tracks reST literal block bodies so field-like examples stay literal.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LiteralBlockTracker {
    pending_literal: bool,
    in_literal: bool,
    block_indent: usize,
}

impl LiteralBlockTracker {
    fn contains_current_line(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        let indent = FieldStart::indentation(line);

        if self.in_literal && indent < self.block_indent && !trimmed.is_empty() {
            self.in_literal = false;
            self.block_indent = 0;
        }

        if self.pending_literal && !trimmed.is_empty() {
            self.pending_literal = false;
            self.in_literal = true;
            self.block_indent = indent;
        }

        self.in_literal
    }

    fn observe_plaintext_line(&mut self, line: &str) {
        if !self.in_literal && starts_literal_block(line.trim_start()) {
            self.pending_literal = true;
        }
    }
}

fn starts_literal_block(line: &str) -> bool {
    let Some(prefix) = line.strip_suffix("::").or_else(|| {
        let (prefix, _language) = line.rsplit_once(' ')?;
        let prefix = prefix.trim_end().strip_suffix("::")?;
        Some(prefix)
    }) else {
        return false;
    };

    let directive = prefix
        .rsplit_once(' ')
        .and_then(|(prefix, directive)| prefix.strip_suffix("..").map(|_| directive));

    !matches!(
        directive,
        Some(
            "attention"
                | "caution"
                | "danger"
                | "error"
                | "hint"
                | "important"
                | "note"
                | "tip"
                | "warning"
                | "admonition"
                | "versionadded"
                | "version-added"
                | "versionchanged"
                | "version-changed"
                | "version-deprecated"
                | "deprecated"
                | "version-removed"
                | "versionremoved"
        )
    )
}

/// The opening line of a field entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldStart<'a> {
    indent: usize,
    kind: FieldKind,
    body: &'a str,
    raw: &'a str,
}

impl<'a> FieldStart<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let trimmed = line.trim_start();
        let after_opening_colon = trimmed.strip_prefix(':')?;
        let (name_and_argument, body) = after_opening_colon.split_once(':')?;
        let name_and_argument = name_and_argument.trim();
        if name_and_argument.is_empty() {
            return None;
        }

        let name_end = name_and_argument
            .find(char::is_whitespace)
            .unwrap_or(name_and_argument.len());
        debug_assert!(name_and_argument.is_char_boundary(name_end));

        let name = &name_and_argument[..name_end];
        let argument = name_and_argument[name_end..].trim();
        let kind = FieldKind::parse(name, argument);
        let indent = Self::indentation(line);

        debug_assert!(line.is_char_boundary(indent));

        Some(Self {
            indent,
            kind,
            body: body.trim_start(),
            raw: line,
        })
    }

    fn indentation(line: &str) -> usize {
        leading_indentation(line).len()
    }
}

/// A recognized field name and argument before the body is collected.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldKind {
    Parameter { name: String, ty: Option<String> },
    ParameterType { name: String },
    Returns { name: Option<String> },
    ReturnType,
    Raises { exception: Option<String> },
    Unsupported,
}

impl FieldKind {
    fn parse(name: &str, argument: &str) -> Self {
        match name {
            "param" | "parameter" | "arg" | "argument" | "key" | "keyword" | "kwarg"
            | "kwparam" => Self::parse_parameter_argument(argument)
                .map(|(ty, name)| Self::Parameter { name, ty })
                .unwrap_or(Self::Unsupported),
            "type" | "paramtype" => Self::parse_parameter_name(argument)
                .map(|name| Self::ParameterType { name })
                .unwrap_or(Self::Unsupported),
            "return" | "returns" => Self::Returns {
                name: Self::parse_parameter_name(argument),
            },
            "rtype" => Self::ReturnType,
            "raises" | "raise" | "except" | "exception" => {
                let exception = argument.trim();
                Self::Raises {
                    exception: (!exception.is_empty()).then(|| exception.to_string()),
                }
            }
            _ => Self::Unsupported,
        }
    }

    fn parse_parameter_argument(argument: &str) -> Option<(Option<String>, String)> {
        let argument = argument.trim();
        if argument.is_empty() {
            return None;
        }

        let name_start = argument
            .char_indices()
            .rev()
            .find_map(|(index, char)| char.is_whitespace().then_some(index + char.len_utf8()));

        // The final whitespace-delimited token is the parameter name.
        if let Some(name_start) = name_start {
            let ty = argument[..name_start].trim();
            let name = argument[name_start..].trim();
            let name = Self::parse_parameter_name(name)?;

            Some(((!ty.is_empty()).then(|| ty.to_string()), name))
        } else {
            Some((None, Self::parse_parameter_name(argument)?))
        }
    }

    fn parse_parameter_name(name: &str) -> Option<String> {
        let name = name.trim().trim_start_matches('*');
        (!name.is_empty()).then(|| name.to_string())
    }
}

/// Accumulates one field's normalized body and raw source.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldBuilder<'a> {
    indent: usize,
    kind: FieldKind,
    body: &'a str,
    raw_lines: Vec<&'a str>,
}

impl<'a> FieldBuilder<'a> {
    fn new(start: FieldStart<'a>) -> Self {
        let FieldStart {
            indent,
            kind,
            body,
            raw,
        } = start;

        Self {
            indent,
            kind,
            body,
            raw_lines: vec![raw],
        }
    }

    fn finish(self) -> Field {
        debug_assert!(!self.raw_lines.is_empty());

        let body = self.normalized_body();
        let raw = self.raw_lines.join("\n");

        match self.kind {
            FieldKind::Parameter { name, ty } => Field::Parameter {
                name,
                ty,
                description: body,
            },
            FieldKind::ParameterType { name } => Field::ParameterType {
                name,
                ty: body,
                raw,
            },
            FieldKind::Returns { name } => Field::Returns {
                name,
                description: body,
            },
            FieldKind::ReturnType => Field::ReturnType { ty: body },
            FieldKind::Raises { exception } => Field::Raises {
                exception,
                description: body,
            },
            FieldKind::Unsupported => Field::Unsupported { raw },
        }
    }

    fn normalized_body(&self) -> String {
        let body_indent = self
            .raw_lines
            .iter()
            .skip(1)
            .filter_map(|line| match line {
                line if line.trim().is_empty() => None,
                line => Some(FieldStart::indentation(line)),
            })
            .min()
            .unwrap_or(0);

        let lines = std::iter::once(self.body.trim_end().to_string())
            .chain(self.raw_lines.iter().skip(1).map(|line| {
                if line.trim().is_empty() {
                    String::new()
                } else {
                    line.get(body_indent..)
                        .unwrap_or_default()
                        .trim_end()
                        .to_string()
                }
            }))
            .collect::<Vec<_>>();

        let Some(start) = lines.iter().position(|line| !line.is_empty()) else {
            return String::new();
        };
        let end = lines
            .iter()
            .rposition(|line| !line.is_empty())
            .map_or(start, |index| index + 1);

        lines[start..end].join("\n")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use insta::assert_snapshot;

    use super::{Formatter, Parser};

    #[test]
    fn parameter_documentation_extracts_parameters() {
        let docstring = r#"
        This is a function description.

        :param str param1: The first parameter description
        :param int param2: The second parameter description
            This is a continuation of param2 description.
        :param param3: A parameter without type annotation
        :returns: The return value description
        :rtype: str
        "#;
        let param_docs = parameter_documentation(docstring);

        assert_eq!(param_docs.len(), 3);
        assert_eq!(
            param_docs.get("param1").expect("param1 should exist"),
            "The first parameter description"
        );
        assert_eq!(
            param_docs.get("param2").expect("param2 should exist"),
            "The second parameter description\nThis is a continuation of param2 description."
        );
        assert_eq!(
            param_docs.get("param3").expect("param3 should exist"),
            "A parameter without type annotation"
        );
    }

    #[test]
    fn parameter_documentation_stops_at_field_boundaries() {
        let docstring = r#"
        :param param: The parameter description
        :type param: bool
        :returns value: The return value description
        :rtype: str
        "#;
        let param_docs = parameter_documentation(docstring);

        assert_eq!(param_docs.len(), 1);
        assert_eq!(
            param_docs.get("param").expect("param should exist"),
            "The parameter description"
        );
    }

    #[test]
    fn parameter_documentation_supports_complex_arguments() {
        let docstring = r#"
        :param list[str] names: The names to process.
        :param **kwargs: Extra keyword arguments.
        "#;
        let param_docs = parameter_documentation(docstring);

        assert_eq!(param_docs.len(), 2);
        assert_eq!(
            param_docs.get("names").expect("names should exist"),
            "The names to process."
        );
        assert_eq!(
            param_docs.get("kwargs").expect("kwargs should exist"),
            "Extra keyword arguments."
        );
    }

    #[test]
    fn parameter_documentation_supports_parameter_aliases() {
        let docstring = r#"
        :parameter first: The first parameter.
        :arg second: The second parameter.
        :argument third: The third parameter.
        :key fourth: The fourth parameter.
        :keyword fifth: The fifth parameter.
        :kwarg sixth: The sixth parameter.
        :kwparam seventh: The seventh parameter.
        "#;
        let param_docs = parameter_documentation(docstring);

        assert_eq!(param_docs.len(), 7);
        assert_eq!(
            param_docs.get("first").expect("first should exist"),
            "The first parameter."
        );
        assert_eq!(
            param_docs.get("second").expect("second should exist"),
            "The second parameter."
        );
        assert_eq!(
            param_docs.get("third").expect("third should exist"),
            "The third parameter."
        );
        assert_eq!(
            param_docs.get("fourth").expect("fourth should exist"),
            "The fourth parameter."
        );
        assert_eq!(
            param_docs.get("fifth").expect("fifth should exist"),
            "The fifth parameter."
        );
        assert_eq!(
            param_docs.get("sixth").expect("sixth should exist"),
            "The sixth parameter."
        );
        assert_eq!(
            param_docs.get("seventh").expect("seventh should exist"),
            "The seventh parameter."
        );
    }

    #[test]
    fn parameter_documentation_ignores_parameters_without_names_after_normalization() {
        let docstring = ":param **: Missing a parameter name.";

        assert!(parameter_documentation(docstring).is_empty());
    }

    #[test]
    fn field_lists_render_as_markdown_sections() {
        let docstring = "\
This is a function description.

:param str param1: The first parameter description
:param param2: The second parameter description
:type param2: int
:returns baz: The return value description
:rtype: str";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        This is a function description.

        ## Parameters
        `param1` (`str`): The first parameter description
        `param2` (`int`): The second parameter description

        ## Returns
        `baz` (`str`): The return value description
        ");
    }

    #[test]
    fn field_lists_keep_indented_field_syntax_in_field_body() {
        let docstring = "\
:param foo:
    :class:`Bar` instance
:param baz: Another parameter";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        ## Parameters
        `foo`: :class:`Bar` instance
        `baz`: Another parameter
        ");

        let param_docs = parameter_documentation(docstring);
        assert_eq!(param_docs.len(), 2);
        assert_eq!(
            param_docs.get("foo").expect("foo should exist"),
            ":class:`Bar` instance"
        );
        assert_eq!(
            param_docs.get("baz").expect("baz should exist"),
            "Another parameter"
        );
    }

    #[test]
    fn field_lists_render_types_with_rest_inline_markup_as_code_spans() {
        let docstring = "\
:param foo: The object.
:type foo: :class:`Bar`
:returns: The new object.
:rtype: :class:`Bar`";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        ## Parameters
        `foo` (`` :class:`Bar` ``): The object.

        ## Returns
        (`` :class:`Bar` ``): The new object.
        ");
    }

    #[test]
    fn field_lists_render_sphinx_keyword_and_paramtype_aliases_as_markdown() {
        let docstring = "\
:kwarg timeout: Maximum wait in seconds.
:paramtype timeout: float
:kwparam retries: Number of retry attempts.
:paramtype retries: int";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        ## Parameters
        `timeout` (`float`): Maximum wait in seconds.
        `retries` (`int`): Number of retry attempts.
        ");
    }

    #[test]
    fn field_lists_ignore_parameters_without_names_after_normalization() {
        let docstring = "\
:param **: Missing a parameter name.
:type **: str";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        :param **: Missing a parameter name.
        :type **: str
        ");

        assert!(parameter_documentation(docstring).is_empty());
    }

    #[test]
    fn field_lists_render_raises_as_markdown_sections() {
        let docstring = "\
Checks a value.

:raises ValueError: If the value is invalid.
    Validation happens before any work starts.
:exception RuntimeError: If the system is unavailable.
:except OSError: If the file cannot be read.
:raise: If validation fails without a concrete type.";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        Checks a value.

        ## Raises
        `ValueError`: If the value is invalid.
            Validation happens before any work starts.
        `RuntimeError`: If the system is unavailable.
        `OSError`: If the file cannot be read.
        If validation fails without a concrete type.
        ");
    }

    #[test]
    fn field_lists_render_block_descriptions_on_new_lines() {
        let docstring = "\
:param example:
    ```python
    if ok:
        do_work()
    ```
:param prompt:
    >>> print('prompt')";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @r#"
        ## Parameters
        `example`:
        ```python
        if ok:
            do_work()
        ```
        `prompt`:
        >>> print('prompt')
        "#);

        let param_docs = parameter_documentation(docstring);
        assert_eq!(
            param_docs.get("example").expect("example should exist"),
            "```python\nif ok:\n    do_work()\n```"
        );
    }

    #[test]
    fn field_lists_in_markdown_fences_are_preserved() {
        let docstring = "\
Example input:

```text
:kwarg timeout: This is sample input
:param **kwargs: This is sample input
:returns bar: This is sample output
```

:param real: Real parameter";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        Example input:

        ```text
        :kwarg timeout: This is sample input
        :param **kwargs: This is sample input
        :returns bar: This is sample output
        ```

        ## Parameters
        `real`: Real parameter
        ");

        let param_docs = parameter_documentation(docstring);
        assert_eq!(param_docs.len(), 1);
        assert_eq!(
            param_docs.get("real").expect("real should exist"),
            "Real parameter"
        );
    }

    #[test]
    fn field_lists_in_doctests_are_preserved() {
        let docstring = "\
Example output:

>>> print(\"field list\")
:kwarg timeout: This is sample output
:param **kwargs: This is sample output

:param real: Real parameter";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        Example output:

        >>> print(\"field list\")
        :kwarg timeout: This is sample output
        :param **kwargs: This is sample output

        ## Parameters
        `real`: Real parameter
        ");

        let param_docs = parameter_documentation(docstring);
        assert_eq!(param_docs.len(), 1);
        assert_eq!(
            param_docs.get("real").expect("real should exist"),
            "Real parameter"
        );
    }

    #[test]
    fn field_lists_in_rest_literal_blocks_are_preserved() {
        let docstring = "\
Example::

:param foo: This is sample input";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        Example::

        :param foo: This is sample input
        ");
        assert!(parameter_documentation(docstring).is_empty());
    }

    #[test]
    fn field_lists_in_rest_code_blocks_are_preserved() {
        let docstring = "\
.. code-block:: python

:param foo: This is sample input";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        .. code-block:: python

        :param foo: This is sample input
        ");
        assert!(parameter_documentation(docstring).is_empty());
    }

    #[test]
    fn field_lists_after_rest_literal_blocks_still_render() {
        let docstring = "\
Example:

::

    :param foo: This is sample input

:param real: Real parameter";

        assert_snapshot!(Formatter::new(docstring).render_field_lists(), @"
        Example:

        ::

            :param foo: This is sample input

        ## Parameters
        `real`: Real parameter
        ");

        let param_docs = parameter_documentation(docstring);
        assert_eq!(param_docs.len(), 1);
        assert_eq!(
            param_docs.get("real").expect("real should exist"),
            "Real parameter"
        );
    }

    fn parameter_documentation(docstring: &str) -> HashMap<String, String> {
        Parser::new(docstring)
            .parameter_documentation()
            .into_iter()
            .map(|parameter| (parameter.name, parameter.description))
            .collect()
    }
}
