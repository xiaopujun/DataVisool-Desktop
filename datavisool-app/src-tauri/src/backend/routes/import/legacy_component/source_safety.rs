//! Produces a parser-only copy that avoids a panic in Biome 0.5.7 when it sees
//! UTF-16 surrogate code units in `\uXXXX` escapes. Replacements keep the byte
//! length unchanged; edits are always applied to the original source.

pub(super) fn parser_source(source: &str) -> Result<String, String> {
    let mut scanner = Scanner { source: source.as_bytes(), shadow: source.as_bytes().to_vec(), index: 0 };
    scanner.code(false)?;
    scanner.verify_remaining_escapes()?;
    String::from_utf8(scanner.shadow).map_err(|_| "脚本源码规范化失败".into())
}

struct Scanner<'a> {
    source: &'a [u8],
    shadow: Vec<u8>,
    index: usize,
}

impl Scanner<'_> {
    fn code(&mut self, template_expression: bool) -> Result<(), String> {
        let mut braces = 0usize;
        let mut regex_allowed = true;
        let mut pending_control = false;
        let mut parens = Vec::new();
        while self.index < self.source.len() {
            let current = self.source[self.index];
            if current.is_ascii_whitespace() {
                self.index += 1;
                continue;
            }
            if current == b'}' && template_expression {
                if braces == 0 {
                    return Ok(());
                }
                braces -= 1;
                self.index += 1;
                regex_allowed = false;
                pending_control = false;
                continue;
            }
            match current {
                b'\'' | b'"' => {
                    self.quoted(current)?;
                    regex_allowed = false;
                    pending_control = false;
                }
                b'`' => {
                    self.template()?;
                    regex_allowed = false;
                    pending_control = false;
                }
                b'/' if self.source.get(self.index + 1) == Some(&b'/') => {
                    self.index += 2;
                    while self.index < self.source.len() && !matches!(self.source[self.index], b'\n' | b'\r') {
                        self.index += 1
                    }
                }
                b'/' if self.source.get(self.index + 1) == Some(&b'*') => {
                    self.index += 2;
                    while self.index + 1 < self.source.len() && &self.source[self.index..self.index + 2] != b"*/" {
                        self.index += 1
                    }
                    self.index = (self.index + 2).min(self.source.len());
                }
                b'/' if regex_allowed => {
                    self.regex();
                    regex_allowed = false;
                    pending_control = false;
                }
                b'/' => {
                    self.index += 1;
                    regex_allowed = true;
                    pending_control = false;
                }
                b'\\' if self.surrogate_escape(self.index).is_some() => {
                    return Err("组件包含标识符中的 UTF-16 代理项转义，无法安全解析".into());
                }
                b'(' => {
                    parens.push(pending_control);
                    self.index += 1;
                    regex_allowed = true;
                    pending_control = false;
                }
                b')' => {
                    regex_allowed = parens.pop().unwrap_or(false);
                    self.index += 1;
                    pending_control = false;
                }
                b'{' => {
                    if template_expression {
                        braces += 1
                    }
                    self.index += 1;
                    regex_allowed = true;
                    pending_control = false;
                }
                b'}' | b']' => {
                    self.index += 1;
                    regex_allowed = false;
                    pending_control = false;
                }
                b'[' => {
                    self.index += 1;
                    regex_allowed = true;
                    pending_control = false;
                }
                b'+' | b'-' => {
                    let doubled = self.source.get(self.index + 1) == Some(&current);
                    self.index += if doubled { 2 } else { 1 };
                    if !doubled {
                        regex_allowed = true;
                    }
                    pending_control = false;
                }
                b';' | b',' | b':' | b'?' | b'=' | b'!' | b'&' | b'|' | b'^' | b'~' | b'*' | b'%' | b'<' | b'>' => {
                    self.index += 1;
                    regex_allowed = true;
                    pending_control = false;
                }
                b'.' => {
                    self.index += 1;
                    regex_allowed = false;
                    pending_control = false;
                }
                _ if is_identifier_start(current) || current >= 0x80 => {
                    let start = self.index;
                    self.identifier()?;
                    let keyword = &self.source[start..self.index];
                    pending_control = matches!(keyword, b"if" | b"while" | b"for" | b"with" | b"switch" | b"catch");
                    regex_allowed = matches!(
                        keyword,
                        b"return"
                            | b"throw"
                            | b"case"
                            | b"delete"
                            | b"void"
                            | b"typeof"
                            | b"instanceof"
                            | b"in"
                            | b"of"
                            | b"yield"
                            | b"await"
                            | b"new"
                            | b"else"
                            | b"do"
                            | b"extends"
                    );
                }
                _ if current.is_ascii_digit() => {
                    self.index += 1;
                    while self.index < self.source.len()
                        && (self.source[self.index].is_ascii_alphanumeric() || matches!(self.source[self.index], b'.' | b'_'))
                    {
                        self.index += 1
                    }
                    regex_allowed = false;
                    pending_control = false;
                }
                _ => {
                    self.index += 1;
                    pending_control = false;
                }
            }
        }
        Ok(())
    }

    fn identifier(&mut self) -> Result<(), String> {
        while self.index < self.source.len() {
            let current = self.source[self.index];
            if is_identifier_continue(current) || current >= 0x80 {
                self.index += 1;
                continue;
            }
            if current == b'\\' {
                if self.surrogate_escape(self.index).is_some() {
                    return Err("组件包含标识符中的 UTF-16 代理项转义，无法安全解析".into());
                }
                if let Some(end) = self.unicode_escape_end(self.index) {
                    self.index = end;
                    continue;
                }
            }
            break;
        }
        Ok(())
    }

    fn quoted(&mut self, quote: u8) -> Result<(), String> {
        self.index += 1;
        while self.index < self.source.len() {
            match self.source[self.index] {
                byte if byte == quote => {
                    self.index += 1;
                    return Ok(());
                }
                b'\\' => self.escape_in_literal()?,
                b'\n' | b'\r' => {
                    self.index += 1;
                    return Ok(());
                }
                _ => self.index += 1,
            }
        }
        Ok(())
    }

    fn template(&mut self) -> Result<(), String> {
        self.index += 1;
        while self.index < self.source.len() {
            match self.source[self.index] {
                b'`' => {
                    self.index += 1;
                    return Ok(());
                }
                b'\\' => self.escape_in_literal()?,
                b'$' if self.source.get(self.index + 1) == Some(&b'{') => {
                    self.index += 2;
                    self.code(true)?;
                    if self.source.get(self.index) == Some(&b'}') {
                        self.index += 1
                    }
                }
                _ => self.index += 1,
            }
        }
        Ok(())
    }

    fn escape_in_literal(&mut self) -> Result<(), String> {
        let start = self.index;
        if let Some((unit, end)) = self.surrogate_escape(start) {
            if (0xdc00..=0xdfff).contains(&unit) {
                return Err("组件包含未配对的 UTF-16 代理项转义，无法安全解析".into());
            }
            let Some((low, pair_end)) = self.surrogate_escape(end) else {
                return Err("组件包含未配对的 UTF-16 代理项转义，无法安全解析".into());
            };
            if !(0xdc00..=0xdfff).contains(&low) {
                return Err("组件包含未配对的 UTF-16 代理项转义，无法安全解析".into());
            }
            let point = 0x10000 + ((unit as u32 - 0xd800) << 10) + (low as u32 - 0xdc00);
            // `\u{xxxxxx}` + a JS line continuation is exactly 12 ASCII bytes,
            // the same size as the original two `\uXXXX` escapes.
            let replacement = format!("\\u{{{point:06x}}}\\\n");
            if replacement.len() != pair_end - start {
                return Err("UTF-16 代理项规范化长度不匹配".into());
            }
            self.shadow[start..pair_end].copy_from_slice(replacement.as_bytes());
            self.index = pair_end;
            return Ok(());
        }
        self.index += 1;
        if self.index < self.source.len() {
            if self.source[self.index] == b'\r' && self.source.get(self.index + 1) == Some(&b'\n') {
                self.index += 2;
            } else {
                self.index += 1;
            }
        }
        Ok(())
    }

    fn regex(&mut self) {
        self.index += 1;
        let mut in_class = false;
        while self.index < self.source.len() {
            match self.source[self.index] {
                b'\\' => {
                    if let Some((_, end)) = self.surrogate_escape(self.index) {
                        // Biome 0.5.7 can abort while parsing surrogate escapes in
                        // regex literals too. The regex body is opaque to the
                        // converter, so replace only in the parser shadow with a
                        // same-length BMP escape; original bytes stay untouched.
                        self.shadow[self.index..end].copy_from_slice(b"\\uE000");
                        self.index = end;
                    } else {
                        self.index = (self.index + 2).min(self.source.len());
                    }
                }
                b'[' => {
                    in_class = true;
                    self.index += 1
                }
                b']' => {
                    in_class = false;
                    self.index += 1
                }
                b'/' if !in_class => {
                    self.index += 1;
                    while self.index < self.source.len() && self.source[self.index].is_ascii_alphabetic() {
                        self.index += 1
                    }
                    return;
                }
                b'\n' | b'\r' => return,
                _ => self.index += 1,
            }
        }
    }

    fn verify_remaining_escapes(&self) -> Result<(), String> {
        let mut index = 0;
        while index < self.shadow.len() {
            if self.shadow[index] != b'\\' {
                index += 1;
                continue;
            }
            let start = index;
            while index < self.shadow.len() && self.shadow[index] == b'\\' {
                index += 1
            }
            let count = index - start;
            if count % 2 == 0 {
                continue;
            }
            let escape = index - 1;
            if Self::surrogate_escape_in(&self.shadow, escape).is_none() {
                continue;
            }
            // Do not trust scanner comment/regex boundaries here: slashes can also be
            // division or string data. Any unnormalized active escape is rejected.
            return Err("组件含未处理的 UTF-16 代理项转义，无法安全解析".into());
        }
        Ok(())
    }

    fn surrogate_escape(&self, start: usize) -> Option<(u16, usize)> {
        Self::surrogate_escape_in(self.source, start)
    }

    fn surrogate_escape_in(bytes: &[u8], start: usize) -> Option<(u16, usize)> {
        if bytes.get(start..start + 2) != Some(b"\\u") {
            return None;
        }
        let digits = bytes.get(start + 2..start + 6)?;
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let value = u16::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
        (0xd800..=0xdfff).contains(&value).then_some((value, start + 6))
    }

    fn unicode_escape_end(&self, start: usize) -> Option<usize> {
        let bytes = self.source;
        if bytes.get(start..start + 2) != Some(b"\\u") {
            return None;
        }
        if bytes.get(start + 2) == Some(&b'{') {
            let end = bytes[start + 3..].iter().position(|byte| *byte == b'}')? + start + 3;
            return Some(end + 1);
        }
        bytes
            .get(start + 2..start + 6)?
            .iter()
            .all(u8::is_ascii_hexdigit)
            .then_some(start + 6)
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte.is_ascii_digit()
}
