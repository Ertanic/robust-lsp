use crate::utils::span_to_range;
use fluent_syntax::ast::{
    CallArguments, Comment, Expression, InlineExpression, Message, NamedArgument, Pattern,
    PatternElement, Term, Variant, VariantKey,
};
use tower_lsp::lsp_types::SemanticToken;
use tree_sitter::Range;

enum SemanticTokenType {
    EnumMember,
    String,
    Comment,
    Number,
    Function,
    Operator,
    Variable,
    Parameter,
}

pub struct AbsoluteToken {
    range: Range,
    token_type: SemanticTokenType,
}

pub struct SemanticAnalyzer<'a> {
    content: &'a str,
}

impl<'a> SemanticAnalyzer<'a> {
    pub fn new(content: &'a str) -> Self {
        Self { content }
    }

    pub fn term_to_semantic(&self, term: Term<&str>) -> Vec<AbsoluteToken> {
        let range = span_to_range(self.content, &term.id.span);
        let mut result = if let Some(comment) = term.comment {
            vec![
                self.comment_to_semantic(comment),
                create_absolute_token(range, SemanticTokenType::EnumMember),
            ]
        } else {
            vec![create_absolute_token(range, SemanticTokenType::EnumMember)]
        };

        term.value
            .elements
            .into_iter()
            .for_each(|el| result.extend(self.pattern_element_to_semantic(el)));

        result
    }

    pub fn message_to_semantic(&self, msg: Message<&str>) -> Vec<AbsoluteToken> {
        let range = span_to_range(self.content, &msg.id.span);
        let mut result = if let Some(comment) = msg.comment {
            vec![
                self.comment_to_semantic(comment),
                create_absolute_token(range, SemanticTokenType::EnumMember),
            ]
        } else {
            vec![create_absolute_token(range, SemanticTokenType::EnumMember)]
        };

        if let Some(Pattern { elements, .. }) = msg.value {
            elements
                .into_iter()
                .for_each(|el| result.extend(self.pattern_element_to_semantic(el)));
        }

        result
    }

    pub fn comment_to_semantic(&self, comment: Comment<&str>) -> AbsoluteToken {
        let range = span_to_range(self.content, &comment.span);
        create_absolute_token(range, SemanticTokenType::Comment)
    }

    fn pattern_element_to_semantic(&self, element: PatternElement<&str>) -> Vec<AbsoluteToken> {
        match element {
            PatternElement::TextElement { span, .. } => {
                let range = span_to_range(self.content, &span);
                vec![create_absolute_token(range, SemanticTokenType::String)]
            }
            PatternElement::Placeable { expression, .. } => self.expr_to_semantic(expression),
        }
    }

    fn expr_to_semantic(&self, expr: Expression<&str>) -> Vec<AbsoluteToken> {
        match expr {
            Expression::Select {
                selector, variants, ..
            } => {
                let mut result = self.inline_expr_to_semantic(selector);

                variants
                    .into_iter()
                    .for_each(|var| result.extend(self.variant_to_semantic(var)));

                result
            }
            Expression::Inline(inline, span) => {
                let range = span_to_range(self.content, &span);
                let mut result = vec![create_absolute_token(range, SemanticTokenType::Operator)];
                result.extend(self.inline_expr_to_semantic(inline));
                result
            }
        }
    }

    fn variant_to_semantic(&self, var: Variant<&str>) -> Vec<AbsoluteToken> {
        let key_range = span_to_range(
            self.content,
            &match var.key {
                VariantKey::Identifier { span, .. } => span,
                VariantKey::NumberLiteral { span, .. } => span,
            },
        );
        let mut result = vec![create_absolute_token(
            key_range,
            SemanticTokenType::Operator,
        )];

        var.value
            .elements
            .into_iter()
            .for_each(|el| result.extend(self.pattern_element_to_semantic(el)));

        result
    }

    fn inline_expr_to_semantic(&self, inline: InlineExpression<&str>) -> Vec<AbsoluteToken> {
        match inline {
            InlineExpression::StringLiteral { span, .. } => {
                let range = span_to_range(self.content, &span);
                vec![create_absolute_token(range, SemanticTokenType::String)]
            }
            InlineExpression::NumberLiteral { span, .. } => {
                let range = span_to_range(self.content, &span);
                vec![create_absolute_token(range, SemanticTokenType::Number)]
            }
            InlineExpression::FunctionReference {
                span, arguments, ..
            } => {
                let range = span_to_range(self.content, &span);
                let mut result = vec![create_absolute_token(range, SemanticTokenType::Function)];
                result.extend(self.func_arguments_to_semantic(arguments));
                result
            }
            InlineExpression::MessageReference { span, .. } => {
                let range = span_to_range(self.content, &span);
                vec![create_absolute_token(range, SemanticTokenType::Variable)]
            }
            InlineExpression::TermReference { span, .. } => {
                let range = span_to_range(self.content, &span);
                vec![create_absolute_token(range, SemanticTokenType::Variable)]
            }
            InlineExpression::VariableReference { span, .. } => {
                let range = span_to_range(self.content, &span);
                vec![create_absolute_token(range, SemanticTokenType::Variable)]
            }
            InlineExpression::Placeable { expression, .. } => self.expr_to_semantic(*expression),
        }
    }

    fn func_arguments_to_semantic(&self, arg: CallArguments<&str>) -> Vec<AbsoluteToken> {
        let named_args = arg
            .named
            .into_iter()
            .flat_map(|named| self.named_func_argument_to_semantic(named));

        arg.positional
            .into_iter()
            .flat_map(|inline| self.inline_expr_to_semantic(inline))
            .chain(named_args)
            .collect()
    }

    fn named_func_argument_to_semantic(
        &self,
        named_arg: NamedArgument<&str>,
    ) -> Vec<AbsoluteToken> {
        let range = span_to_range(self.content, &named_arg.name.span);
        let mut result = vec![create_absolute_token(range, SemanticTokenType::Parameter)];
        result.extend(self.inline_expr_to_semantic(named_arg.value));
        result
    }
}

fn create_absolute_token(range: Range, token_type: SemanticTokenType) -> AbsoluteToken {
    AbsoluteToken { range, token_type }
}

pub fn to_relative_semantic_tokens(tokens: Vec<AbsoluteToken>) -> Vec<SemanticToken> {
    let mut tokens = tokens;
    tokens.sort_by(|a, b| {
        a.range
            .start_point
            .row
            .cmp(&b.range.start_point.row)
            .then(a.range.start_point.column.cmp(&b.range.start_point.column))
    });

    let mut relative_tokens = Vec::with_capacity(tokens.len());
    let mut prev_line = 0;
    let mut prev_col = 0;

    for token in tokens {
        let line = token.range.start_point.row as u32;
        let col = token.range.start_point.column as u32;
        let length = (token.range.end_byte - token.range.start_byte) as u32;

        let delta_line = line - prev_line;
        let delta_start = if delta_line > 0 { col } else { col - prev_col };

        relative_tokens.push(SemanticToken {
            delta_line,
            delta_start,
            length,
            token_type: token.token_type as u32,
            token_modifiers_bitset: 0,
        });

        prev_line = line;
        prev_col = col;
    }

    relative_tokens
}
