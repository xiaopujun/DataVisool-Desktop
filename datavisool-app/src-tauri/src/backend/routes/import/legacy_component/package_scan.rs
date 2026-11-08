use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

const BUFFER_SIZE: usize = 8192;
const WINDOW_SIZE: usize = 128;
const LEGACY_MARKERS: &[&[u8]] = &[
    b"LightChaserInterface",
    b"LightChaserUtil",
    b"LightChaserUI",
    b"LCPUI",
    b"LCPSDK",
    b"LCPTypes",
    b"__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__",
    b"__LCP_REMOTE_COMPONENT_BASE_URL_MAP__",
    b".lcp-",
    b"--lcp-",
    b".lc-dialog",
    b".cm-s-lcp-",
    b"lcp-",
    b"lc-dialog",
];

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ScanOutcome {
    Clean,
    Candidate,
    Uncertain,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceKind {
    JavaScript,
    Css,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Code,
    SingleQuoted,
    DoubleQuoted,
    Template,
    LineComment,
    BlockComment,
    Regex,
}

/// Scans oversized source without allocating it. Ambiguous escapes are reported as uncertain.
pub(super) fn scan_large_source(path: &Path, kind: SourceKind) -> Result<ScanOutcome, String> {
    let file = File::open(path).map_err(|e| format!("读取组件文件失败：{e}"))?;
    let mut reader = BufReader::with_capacity(BUFFER_SIZE, file);
    let mut block = [0u8; BUFFER_SIZE];
    let mut mode = Mode::Code;
    let mut escaped = false;
    let mut slash_pending = false;
    let mut block_star = false;
    let mut regex_allowed = true;
    let mut regex_ambiguous = false;
    let mut regex_class = false;
    let mut regex_escaped = false;
    let mut word = Vec::with_capacity(32);
    let mut class_name_pending = false;
    let mut class_name_value = false;
    let mut class_name_string = false;
    let mut class_attribute_state = 0u8;
    let mut window = Vec::with_capacity(WINDOW_SIZE);
    let mut utf8_tail = Vec::with_capacity(4);

    loop {
        let count = reader.read(&mut block).map_err(|e| format!("扫描组件文件失败：{e}"))?;
        if count == 0 {
            break;
        }
        if !validate_utf8_chunk(&mut utf8_tail, &block[..count]) {
            return Err("组件文件不是 UTF-8".into());
        }
        for &byte in &block[..count] {
            if mode == Mode::LineComment {
                if byte == b'\n' || byte == b'\r' {
                    if append_and_check(&mut window, &mut class_attribute_state, b' ') != ScanOutcome::Clean {
                        return Ok(ScanOutcome::Candidate);
                    }
                    mode = Mode::Code;
                }
                continue;
            }
            if mode == Mode::BlockComment {
                if block_star && byte == b'/' {
                    if append_and_check(&mut window, &mut class_attribute_state, b' ') != ScanOutcome::Clean {
                        return Ok(ScanOutcome::Candidate);
                    }
                    mode = Mode::Code;
                    block_star = false;
                } else {
                    block_star = byte == b'*';
                }
                continue;
            }

            if mode == Mode::Regex {
                if append_and_check(&mut window, &mut class_attribute_state, byte) != ScanOutcome::Clean {
                    return Ok(ScanOutcome::Candidate);
                }
                if regex_escaped {
                    regex_escaped = false;
                } else if byte == b'\\' {
                    regex_escaped = true;
                } else if byte == b'[' {
                    regex_class = true;
                } else if byte == b']' {
                    regex_class = false;
                } else if byte == b'/' && !regex_class {
                    mode = Mode::Code;
                    regex_allowed = false;
                    regex_ambiguous = false;
                }
                continue;
            }

            if mode == Mode::Code && slash_pending {
                slash_pending = false;
                if kind == SourceKind::JavaScript && byte == b'/' {
                    mode = Mode::LineComment;
                    continue;
                }
                if byte == b'*' {
                    mode = Mode::BlockComment;
                    block_star = false;
                    continue;
                }
                flush_word(&mut word, &mut regex_allowed, &mut regex_ambiguous, &mut class_name_pending);
                if kind == SourceKind::JavaScript && regex_ambiguous {
                    return Ok(ScanOutcome::Uncertain);
                }
                if kind == SourceKind::JavaScript && regex_allowed {
                    if append_and_check(&mut window, &mut class_attribute_state, b'/') != ScanOutcome::Clean {
                        return Ok(ScanOutcome::Candidate);
                    }
                    mode = Mode::Regex;
                    regex_class = false;
                    regex_escaped = false;
                    // The byte after the opening slash is the first regex byte.
                    if byte == b'\\' {
                        regex_escaped = true;
                    } else if byte == b'[' {
                        regex_class = true;
                    } else if byte == b'/' {
                        mode = Mode::Code;
                        regex_allowed = false;
                    }
                    if append_and_check(&mut window, &mut class_attribute_state, byte) != ScanOutcome::Clean {
                        return Ok(ScanOutcome::Candidate);
                    }
                    continue;
                }
                if append_and_check(&mut window, &mut class_attribute_state, b'/') == ScanOutcome::Candidate {
                    return Ok(ScanOutcome::Candidate);
                }
                if kind == SourceKind::JavaScript {
                    regex_allowed = true;
                    regex_ambiguous = false;
                }
            }

            if mode == Mode::Code {
                match byte {
                    b'/' => {
                        slash_pending = true;
                        continue;
                    }
                    b'\'' => {
                        class_name_string = kind == SourceKind::JavaScript && class_name_value;
                        mode = Mode::SingleQuoted;
                        escaped = false;
                    }
                    b'"' => {
                        class_name_string = kind == SourceKind::JavaScript && class_name_value;
                        mode = Mode::DoubleQuoted;
                        escaped = false;
                    }
                    b'`' => {
                        class_name_string = kind == SourceKind::JavaScript && class_name_value;
                        mode = Mode::Template;
                        escaped = false;
                    }
                    _ => {}
                }
                if kind == SourceKind::JavaScript {
                    update_regex_context(byte, &mut word, &mut regex_allowed, &mut regex_ambiguous, &mut class_name_pending, &mut class_name_value);
                }
            } else if escaped {
                escaped = false;
            } else if byte == b'\\' {
                if class_name_string {
                    return Ok(ScanOutcome::Uncertain);
                }
                escaped = true;
            } else if (mode == Mode::SingleQuoted && byte == b'\'')
                || (mode == Mode::DoubleQuoted && byte == b'"')
                || (mode == Mode::Template && byte == b'`')
            {
                mode = Mode::Code;
                if class_name_string {
                    class_name_string = false;
                    class_name_value = false;
                }
                if kind == SourceKind::JavaScript {
                    regex_allowed = false;
                    regex_ambiguous = false;
                }
            }

            match append_and_check(&mut window, &mut class_attribute_state, byte) {
                ScanOutcome::Candidate => return Ok(ScanOutcome::Candidate),
                ScanOutcome::Uncertain => return Ok(ScanOutcome::Uncertain),
                ScanOutcome::Clean => {}
            }
        }
    }

    if !utf8_tail.is_empty() {
        return Err("组件文件不是 UTF-8".into());
    }
    if slash_pending {
        if kind == SourceKind::JavaScript && regex_ambiguous {
            return Ok(ScanOutcome::Uncertain);
        }
        if regex_allowed && kind == SourceKind::JavaScript {
            return Ok(ScanOutcome::Uncertain);
        }
        if append_and_check(&mut window, &mut class_attribute_state, b'/') != ScanOutcome::Clean {
            return Ok(ScanOutcome::Candidate);
        }
    }
    Ok(ScanOutcome::Clean)
}

fn flush_word(word: &mut Vec<u8>, regex_allowed: &mut bool, regex_ambiguous: &mut bool, class_name_pending: &mut bool) {
    if word.is_empty() {
        return;
    }
    *regex_allowed = matches!(
        word.as_slice(),
        b"return"
            | b"throw"
            | b"case"
            | b"delete"
            | b"void"
            | b"typeof"
            | b"new"
            | b"yield"
            | b"await"
            | b"else"
            | b"do"
            | b"instanceof"
            | b"in"
            | b"of"
    );
    *regex_ambiguous = false;
    *class_name_pending = word.as_slice() == b"className";
    word.clear();
}

fn update_regex_context(
    byte: u8,
    word: &mut Vec<u8>,
    regex_allowed: &mut bool,
    regex_ambiguous: &mut bool,
    class_name_pending: &mut bool,
    class_name_value: &mut bool,
) {
    if byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' {
        if word.len() < 32 {
            word.push(byte);
        }
        *regex_allowed = false;
        *regex_ambiguous = false;
        return;
    }
    flush_word(word, regex_allowed, regex_ambiguous, class_name_pending);
    match byte {
        b':' if *class_name_pending => {
            *class_name_value = true;
            *class_name_pending = false;
            *regex_allowed = true;
            *regex_ambiguous = false;
        }
        b' ' | b'\t' | b'\r' | b'\n' => {}
        b')' | b'}' => {
            *regex_allowed = false;
            *regex_ambiguous = true;
            *class_name_pending = false;
        }
        b']' | b'.' => {
            *regex_allowed = false;
            *regex_ambiguous = false;
            *class_name_pending = false;
        }
        b'+' | b'-' => {
            *regex_allowed = false;
            *regex_ambiguous = true;
            *class_name_pending = false;
        }
        b'(' | b'[' | b'{' | b'=' | b',' | b';' | b'!' | b'?' | b'&' | b'|' | b'*' | b'%' | b'^' | b'~' | b'<' | b'>' => {
            *regex_allowed = true;
            *regex_ambiguous = false;
            *class_name_pending = false;
            if byte == b';' || byte == b',' || byte == b'{' {
                *class_name_value = false;
            }
        }
        _ => {
            *regex_allowed = true;
            *regex_ambiguous = false;
            *class_name_pending = false;
        }
    }
}

fn validate_utf8_chunk(tail: &mut Vec<u8>, chunk: &[u8]) -> bool {
    if tail.is_empty() {
        match std::str::from_utf8(chunk) {
            Ok(_) => true,
            Err(error) if error.error_len().is_some() => false,
            Err(error) => {
                tail.extend_from_slice(&chunk[error.valid_up_to()..]);
                tail.len() <= 3
            }
        }
    } else {
        let split = tail.len();
        tail.extend_from_slice(chunk);
        match std::str::from_utf8(tail) {
            Ok(_) => {
                tail.clear();
                true
            }
            Err(error) if error.error_len().is_some() => false,
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > split {
                    tail.drain(..valid);
                }
                tail.len() <= 3
            }
        }
    }
}

fn append_and_check(window: &mut Vec<u8>, class_attribute_state: &mut u8, byte: u8) -> ScanOutcome {
    window.push(byte);
    if window.len() > WINDOW_SIZE * 2 {
        window.drain(..WINDOW_SIZE);
    }
    update_class_attribute_state(class_attribute_state, byte);
    if matches!(byte, b'e' | b'l' | b'I' | b'K' | b's' | b'_' | b'-' | b'g')
        && LEGACY_MARKERS
            .iter()
            .filter(|marker| marker.last() == Some(&byte))
            .any(|marker| window.ends_with(marker))
    {
        return ScanOutcome::Candidate;
    }

    if active_unicode_escape(window) {
        return ScanOutcome::Uncertain;
    }
    if byte == b'\\' && *class_attribute_state == 8 {
        return ScanOutcome::Candidate;
    }
    if byte == b'\\' && escaped_style_candidate(window) {
        return ScanOutcome::Candidate;
    }
    ScanOutcome::Clean
}

fn update_class_attribute_state(state: &mut u8, byte: u8) {
    *state = match *state {
        0 => {
            if byte == b'[' {
                1
            } else {
                0
            }
        }
        1 if byte.is_ascii_whitespace() => 1,
        1 if byte == b'c' => 2,
        2 if byte == b'l' => 3,
        3 if byte == b'a' => 4,
        4 if byte == b's' => 5,
        5 if byte == b's' => 6,
        6 if byte.is_ascii_whitespace() => 6,
        6 if matches!(byte, b'=' | b'~' | b'*' | b'^' | b'$' | b'|') => {
            if byte == b'=' {
                8
            } else {
                7
            }
        }
        7 if byte == b'=' => 8,
        8 if byte == b']' => 0,
        8 => 8,
        _ if byte == b'[' => 1,
        _ => 0,
    };
}

fn active_unicode_escape(window: &[u8]) -> bool {
    if window.last() != Some(&b'u') {
        return false;
    }
    let slash_count = window.iter().rev().skip(1).take_while(|&&byte| byte == b'\\').count();
    slash_count % 2 == 1
}

fn escaped_style_candidate(window: &[u8]) -> bool {
    if window.ends_with(b".\\") || window.ends_with(b"--\\") {
        return true;
    }
    let Some(slash) = window.iter().rposition(|&byte| byte == b'\\') else {
        return false;
    };
    let prefix = &window[..slash];
    if let Some(class_name) = prefix.windows(b"className".len()).rposition(|part| part == b"className") {
        let suffix = &prefix[class_name + b"className".len()..];
        if suffix.contains(&b':') && (suffix.contains(&b'\'') || suffix.contains(&b'"')) {
            return true;
        }
    }
    let token_start = prefix
        .iter()
        .rposition(|&byte| byte.is_ascii_whitespace() || matches!(byte, b'\'' | b'"' | b'(' | b')' | b'{' | b'}' | b';' | b','))
        .map_or(0, |index| index + 1);
    let token = &prefix[token_start..];
    token.starts_with(b".")
        || token.starts_with(b"--")
        || (token.starts_with(b"[class") && token.contains(&b'='))
        || (token.starts_with(b"className") && token.contains(&b':'))
}
