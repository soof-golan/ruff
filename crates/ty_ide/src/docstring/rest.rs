use ruff_python_trivia::leading_indentation;
use ruff_source_file::UniversalNewlines;

/// Extracts parameter documentation from reST field lists in a docstring.
pub(super) struct Parser<'a> {
    docstring: &'a str,
}

impl<'a> Parser<'a> {
    pub(super) const fn new(docstring: &'a str) -> Self {
        Self { docstring }
    }

    pub(super) fn parameter_documentation(&self) -> Vec<ParameterDocumentation> {
        let mut parameters = Vec::new();
        let mut lines = self.docstring.universal_newlines().peekable();

        while let Some(line) = lines.next() {
            let Some(field) = FieldStart::parse(line.as_str()) else {
                continue;
            };

            let FieldKind::Parameter { name } = field.kind else {
                continue;
            };

            let mut description_lines = vec![field.body.to_string()];

            while let Some(line) = lines.peek() {
                let line = line.as_str();
                if FieldStart::parse(line).is_some() {
                    break;
                }

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    description_lines.push(String::new());
                    lines.next();
                    continue;
                }

                if FieldStart::indentation(line) > field.indent {
                    description_lines.push(trimmed.to_string());
                    lines.next();
                } else {
                    break;
                }
            }

            let description = description_lines.join("\n").trim().to_string();
            if !description.is_empty() {
                parameters.push(ParameterDocumentation { name, description });
            }
        }

        parameters
    }
}

/// Parameter documentation extracted from a reST field list.
pub(super) struct ParameterDocumentation {
    pub(super) name: String,
    pub(super) description: String,
}

/// The opening line of a field entry.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldStart<'a> {
    indent: usize,
    kind: FieldKind,
    body: &'a str,
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
        })
    }

    fn indentation(line: &str) -> usize {
        leading_indentation(line).len()
    }
}

/// A recognized field name and argument before the body is collected.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldKind {
    Parameter { name: String },
    Other,
}

impl FieldKind {
    fn parse(name: &str, argument: &str) -> Self {
        match name {
            "param" | "parameter" | "arg" | "argument" | "key" | "keyword" | "kwarg"
            | "kwparam" => Self::parse_parameter_argument(argument)
                .map(|name| Self::Parameter { name })
                .unwrap_or(Self::Other),
            _ => Self::Other,
        }
    }

    fn parse_parameter_argument(argument: &str) -> Option<String> {
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
            let name = argument[name_start..].trim();
            let name = Self::parse_parameter_name(name)?;

            Some(name)
        } else {
            Self::parse_parameter_name(argument)
        }
    }

    fn parse_parameter_name(name: &str) -> Option<String> {
        let name = name.trim().trim_start_matches('*');
        (!name.is_empty()).then(|| name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Parser;

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

    fn parameter_documentation(docstring: &str) -> HashMap<String, String> {
        Parser::new(docstring)
            .parameter_documentation()
            .into_iter()
            .map(|parameter| (parameter.name, parameter.description))
            .collect()
    }
}
