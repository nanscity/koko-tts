use regex::Regex;

/// Cleans raw text (such as copied from Wikipedia, web pages, or Markdown documents)
/// before chunking and speech synthesis:
/// - Converts Markdown links `[text](url)` to just `text` (handles URLs with nested parentheses).
/// - Removes citation markers: `[1]`, `[[1]](url)`, `[note 2]`, `[citation needed]`, etc.
/// - Removes bare URLs (`http://`, `https://`, `www.`).
/// - Strips Markdown formatting characters (`#`, `*`, `_`, `` ` ``, `>`).
/// - Removes emojis and non-speech symbols.
/// - Transliterates accented / Slavic / European Latin diacritics (e.g. `š` -> `sh` so "Duško" is pronounced naturally).
/// - Collapses whitespace and normalizes punctuation.
pub fn clean_text(input: &str) -> String {
    let mut text = input.to_string();

    // 1. Convert Markdown links [label](url) where url may contain balanced parentheses like (writer)
    text = clean_markdown_links(&text);

    // 2. Remove standalone citation markers like [1], [2], [12], [note 1], [citation needed], [edit]
    if let Ok(re) = Regex::new(r"(?i)\[(?:(?:\d+|note\s*\d+|nb\s*\d+|citation\s+needed|clarification\s+needed|edit|when\?|who\?))\]") {
        text = re.replace_all(&text, "").to_string();
    }
    // Also remove double brackets like [[1]], [[2]]
    if let Ok(re) = Regex::new(r"\[\[(?:\d+|[^\]]+)\]\]") {
        text = re.replace_all(&text, "").to_string();
    }

    // 3. Remove bare URLs: http://..., https://..., www....
    if let Ok(re) = Regex::new(r"https?://\S+|www\.\S+") {
        text = re.replace_all(&text, "").to_string();
    }

    // 4. Remove Markdown formatting symbols
    // Headers: # at start of line
    if let Ok(re) = Regex::new(r"(?m)^#+\s*") {
        text = re.replace_all(&text, "").to_string();
    }
    // Bold / italic asterisks and underscores: **text**, *text*, __text__, _text_
    text = text.replace("**", "").replace("__", "");
    text = text.replace('*', "").replace('_', " ");
    // Inline code backticks: `code`
    text = text.replace('`', "");
    // Blockquote > at start of line
    if let Ok(re) = Regex::new(r"(?m)^>\s*") {
        text = re.replace_all(&text, "").to_string();
    }
    // Bullet points at start of line: "* ", "- ", "+ "
    if let Ok(re) = Regex::new(r"(?m)^[\*\-\+]\s+") {
        text = re.replace_all(&text, "").to_string();
    }

    // 5. Transliterate common diacritics / non-ASCII characters to readable English phonetics
    text = transliterate_diacritics(&text);

    // 6. Strip emojis and special Unicode symbols
    text = text.chars().filter(|&c| !is_emoji(c)).collect();

    // 7. Normalize whitespace and punctuation spacing
    if let Ok(re) = Regex::new(r"[ \t]+") {
        text = re.replace_all(&text, " ").to_string();
    }
    if let Ok(re) = Regex::new(r"\s+([,.:;?!])") {
        text = re.replace_all(&text, "$1").to_string();
    }
    if let Ok(re) = Regex::new(r"[,.:;?!]{3,}") {
        text = re.replace_all(&text, "...").to_string();
    }

    // Clean up empty lines or lines with dangling colons
    let cleaned_lines: Vec<&str> = text
        .lines()
        .map(|l| l.trim().trim_end_matches(':'))
        .filter(|l| !l.is_empty())
        .collect();

    cleaned_lines.join("\n").trim().to_string()
}

/// Parses Markdown links `[text](url)` and extracts `text`, handling URLs with balanced parentheses.
fn clean_markdown_links(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '[' {
            // Find closing ']'
            let mut j = i + 1;
            let mut bracket_depth = 1;
            while j < len && bracket_depth > 0 {
                if chars[j] == '[' {
                    bracket_depth += 1;
                } else if chars[j] == ']' {
                    bracket_depth -= 1;
                }
                j += 1;
            }

            if bracket_depth == 0 && j < len && chars[j] == '(' {
                let label: String = chars[i + 1..j - 1].iter().collect();
                // Find matching ')' for (url) with balanced parentheses
                let mut k = j + 1;
                let mut paren_depth = 1;
                while k < len && paren_depth > 0 {
                    if chars[k] == '(' {
                        paren_depth += 1;
                    } else if chars[k] == ')' {
                        paren_depth -= 1;
                    }
                    k += 1;
                }

                if paren_depth == 0 {
                    let clean_label = label.trim().trim_matches(|c| c == '[' || c == ']');
                    let is_citation = clean_label.chars().all(|c| c.is_ascii_digit())
                        || clean_label.starts_with("note")
                        || clean_label.starts_with("cite");

                    if !is_citation {
                        result.push_str(label.trim());
                    }
                    i = k;
                    continue;
                }
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Transliterates European / Slavic / Latin diacritics into plain phonetic equivalents
/// so the English TTS model pronounces them naturally without emitting question marks or spelling out characters.
fn transliterate_diacritics(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            'š' => out.push_str("sh"),
            'Š' => out.push_str("Sh"),
            'č' | 'ć' => out.push_str("ch"),
            'Č' | 'Ć' => out.push_str("Ch"),
            'ž' => out.push_str("zh"),
            'Ž' => out.push_str("Zh"),
            'đ' => out.push_str("dj"),
            'Đ' => out.push_str("Dj"),
            'é' | 'è' | 'ê' | 'ë' => out.push('e'),
            'É' | 'È' | 'Ê' | 'Ë' => out.push('E'),
            'á' | 'à' | 'â' | 'ä' | 'å' => out.push('a'),
            'Á' | 'À' | 'Â' | 'Ä' | 'Å' => out.push('A'),
            'ó' | 'ò' | 'ô' | 'ö' => out.push('o'),
            'Ó' | 'Ò' | 'Ô' | 'Ö' => out.push('O'),
            'ú' | 'ù' | 'û' | 'ü' => out.push('u'),
            'Ú' | 'Ù' | 'Û' | 'Ü' => out.push('U'),
            'í' | 'ì' | 'î' | 'ï' => out.push('i'),
            'Í' | 'Ì' | 'Î' | 'Ï' => out.push('I'),
            'ñ' => out.push_str("ny"),
            'Ñ' => out.push_str("Ny"),
            'ç' => out.push('s'),
            'Ç' => out.push('S'),
            _ => out.push(c),
        }
    }
    out
}

fn is_emoji(c: char) -> bool {
    let u = c as u32;
    matches!(u,
        0x1F600..=0x1F64F // Emoticons
        | 0x1F300..=0x1F5FF // Misc Symbols and Pictographs
        | 0x1F680..=0x1F6FF // Transport and Map
        | 0x1F700..=0x1F77F // Alchemical
        | 0x1F780..=0x1F7FF // Geometric Shapes
        | 0x1F800..=0x1F8FF // Supplemental Arrows
        | 0x1F900..=0x1F9FF // Supplemental Symbols and Pictographs
        | 0x1FA00..=0x1FA6F // Chess Symbols
        | 0x1FA70..=0x1FAFF // Symbols and Pictographs Extended-A
        | 0x2600..=0x26FF   // Misc symbols
        | 0x2700..=0x27BF   // Dingbats
        | 0xFE00..=0xFE0F   // Variation Selectors
    )
}
