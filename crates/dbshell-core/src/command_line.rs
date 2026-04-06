use crate::error::{DbError, Result};
use crate::pipeline::{PipeStage, Pipeline, PushdownCapability};
use crate::tool_kind::{ToolArgs, ToolCall, ToolKind};
use crate::vfs_path::VfsPath;

#[derive(Debug)]
pub struct CommandLine {
    pub groups: Vec<CommandGroup>,
}

#[derive(Debug)]
pub struct CommandGroup {
    pub pipeline: Pipeline,
    pub separator: Separator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Separator {
    /// `;` or newline — wait for this pipeline to finish, then run the next.
    Sequential,
    /// `&` — run this pipeline in the background, continue immediately.
    Background,
    /// End of input.
    End,
}

/// Raw token produced by the tokenizer.
#[derive(Debug, Clone, PartialEq)]
enum Token {
    Word(String),
    Pipe,      // |
    Semi,      // ;
    Ampersand, // &
    Append,    // >>
    Overwrite, // >
}

impl CommandLine {
    /// Parse a raw input string into a CommandLine.
    pub fn parse(input: &str) -> Result<Self> {
        let tokens = tokenize(input)?;
        if tokens.is_empty() {
            return Err(DbError::ParseError("empty input".into()));
        }
        parse_tokens(tokens)
    }
}

/// Tokenize input, respecting single and double quotes.
fn tokenize(input: &str) -> Result<Vec<Token>> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();
    let mut current_word = String::new();

    while let Some(&ch) = chars.peek() {
        match ch {
            '\'' | '"' => {
                let quote = ch;
                chars.next();
                // Consume until matching quote
                loop {
                    match chars.next() {
                        Some(c) if c == quote => break,
                        Some(c) => current_word.push(c),
                        None => {
                            return Err(DbError::ParseError(format!("unterminated {quote} quote")))
                        }
                    }
                }
            }
            '|' => {
                flush_word(&mut current_word, &mut tokens);
                chars.next();
                tokens.push(Token::Pipe);
            }
            ';' => {
                flush_word(&mut current_word, &mut tokens);
                chars.next();
                tokens.push(Token::Semi);
            }
            '&' => {
                flush_word(&mut current_word, &mut tokens);
                chars.next();
                tokens.push(Token::Ampersand);
            }
            '>' => {
                flush_word(&mut current_word, &mut tokens);
                chars.next();
                if chars.peek() == Some(&'>') {
                    chars.next();
                    tokens.push(Token::Append);
                } else {
                    tokens.push(Token::Overwrite);
                }
            }
            ' ' | '\t' | '\n' | '\r' => {
                flush_word(&mut current_word, &mut tokens);
                chars.next();
            }
            _ => {
                current_word.push(ch);
                chars.next();
            }
        }
    }
    flush_word(&mut current_word, &mut tokens);
    Ok(tokens)
}

fn flush_word(word: &mut String, tokens: &mut Vec<Token>) {
    if !word.is_empty() {
        tokens.push(Token::Word(std::mem::take(word)));
    }
}

/// Parse tokens into a CommandLine. Pipes bind tighter than ; and &.
fn parse_tokens(tokens: Vec<Token>) -> Result<CommandLine> {
    let mut groups = Vec::new();

    // Split on ; and & to get pipeline groups
    let mut current_pipeline_tokens: Vec<Token> = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Semi => {
                if !current_pipeline_tokens.is_empty() {
                    let pipeline = parse_pipeline(current_pipeline_tokens)?;
                    groups.push(CommandGroup {
                        pipeline,
                        separator: Separator::Sequential,
                    });
                    current_pipeline_tokens = Vec::new();
                }
            }
            Token::Ampersand => {
                if !current_pipeline_tokens.is_empty() {
                    let pipeline = parse_pipeline(current_pipeline_tokens)?;
                    groups.push(CommandGroup {
                        pipeline,
                        separator: Separator::Background,
                    });
                    current_pipeline_tokens = Vec::new();
                }
            }
            other => {
                current_pipeline_tokens.push(other.clone());
            }
        }
        i += 1;
    }

    // Last group
    if !current_pipeline_tokens.is_empty() {
        let pipeline = parse_pipeline(current_pipeline_tokens)?;
        groups.push(CommandGroup {
            pipeline,
            separator: Separator::End,
        });
    }

    // Validate: transaction control must be standalone
    for group in &groups {
        let stages = &group.pipeline.stages;
        if stages.len() == 1 && stages[0].tool.kind.is_transaction_control() {
            continue; // ok: standalone
        }
        for stage in stages {
            if stage.tool.kind.is_transaction_control() {
                return Err(DbError::ParseError(format!(
                    "{} must be a standalone command, not part of a pipeline",
                    stage.tool.name
                )));
            }
        }
    }

    Ok(CommandLine { groups })
}

/// Parse a sequence of tokens (no ; or & — just pipes) into a Pipeline.
fn parse_pipeline(tokens: Vec<Token>) -> Result<Pipeline> {
    // Split by Pipe to get stages
    let mut stage_token_groups: Vec<Vec<Token>> = vec![Vec::new()];

    for token in tokens {
        match token {
            Token::Pipe => {
                stage_token_groups.push(Vec::new());
            }
            other => {
                stage_token_groups.last_mut().unwrap().push(other);
            }
        }
    }

    let mut stages = Vec::new();
    for stage_tokens in stage_token_groups {
        if stage_tokens.is_empty() {
            return Err(DbError::ParseError("empty pipe stage".into()));
        }
        let stage = parse_stage(stage_tokens)?;
        stages.push(stage);
    }

    Ok(Pipeline { stages })
}

/// Parse tokens for a single pipe stage into a PipeStage.
fn parse_stage(tokens: Vec<Token>) -> Result<PipeStage> {
    let mut words = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Token::Word(w) => words.push(w.clone()),
            Token::Append | Token::Overwrite => {
                // Skip redirect target — redirect semantics are Phase 4 (echo/write tools)
                if i + 1 < tokens.len() && matches!(&tokens[i + 1], Token::Word(_)) {
                    i += 1;
                }
            }
            _ => {
                return Err(DbError::ParseError(format!(
                    "unexpected token in stage: {tokens:?}"
                )));
            }
        }
        i += 1;
    }

    if words.is_empty() {
        return Err(DbError::ParseError("empty stage".into()));
    }

    let tool_name = &words[0];
    let kind = ToolKind::from_name(tool_name)
        .ok_or_else(|| DbError::ParseError(format!("unknown tool: {tool_name}")))?;

    // Parse path and flags from remaining words
    let mut path = None;
    let mut flags = std::collections::HashMap::new();
    let mut positional = Vec::new();

    let mut j = 1;
    while j < words.len() {
        let w = &words[j];
        if w.starts_with('-') && w.len() > 1 {
            let flag_name = w.clone();
            // Check if next word is the flag value
            if j + 1 < words.len()
                && !words[j + 1].starts_with('-')
                && !words[j + 1].starts_with('/')
            {
                flags.insert(flag_name, words[j + 1].clone());
                j += 2;
                continue;
            } else {
                flags.insert(flag_name, String::new());
            }
        } else if w.starts_with('/') && path.is_none() {
            path = Some(VfsPath::parse(w)?);
        } else {
            positional.push(w.clone());
        }
        j += 1;
    }

    let pushdown = match kind {
        ToolKind::Head => {
            let count = flags
                .get("-n")
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(10);
            PushdownCapability::Limit { count }
        }
        ToolKind::Tail => {
            let count = flags
                .get("-n")
                .and_then(|v| {
                    // tail -n +N means offset N-1
                    let s = v.strip_prefix('+').unwrap_or(v);
                    s.parse::<u64>().ok()
                })
                .unwrap_or(10);
            PushdownCapability::Offset { count }
        }
        ToolKind::Grep => {
            let pattern = positional.first().cloned().unwrap_or_default();
            PushdownCapability::GrepFilter { pattern }
        }
        _ => PushdownCapability::None,
    };

    let tool = ToolCall {
        name: tool_name.clone(),
        kind,
        path,
        args: ToolArgs { flags, positional },
        stdin: None,
    };

    Ok(PipeStage { tool, pushdown })
}
