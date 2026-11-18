//! The R10 side of the memory item quarantine (#114; SEC-24a, SEC-24b; ADR 0089).
//!
//! Every item write (`mem.item.write`, and its Rust-only step `mem.item.upsert`) has the param
//! `textQuarantine`, which the core computes from `:text` with [`is_quarantined`]. The caller can
//! not send it ([`super::catalog::check_params`]). So neither the brain nor the `bottle-mcp`
//! `memory.write` tool can store an injection item as `active`. The brain runs its own check too
//! (`src/brain/components/memory-quarantine.ts`) and sends `quarantined`; either one quarantines.
//!
//! The patterns are the four Table RTR-INJ categories of the router step (the same expressions
//! as `router-injection.ts`, #117) plus two memory patterns for indirect injection (SG-INJ-7).
//! Both sides run the golden sets `tests/fixtures/memory-adversarial/items.json` and
//! `tests/fixtures/router-adversarial/utterances.json`.
//!
//! Normalization: NFKC (the same as the TS check), then the zero-width and soft-hyphen code
//! points go, the text folds to lower case, and each run of white space becomes one space.

use std::sync::OnceLock;

use regex::Regex;

/// Optional filler words between a verb and its object ("ignore all of the previous ...").
const FILL: &str = r"(?:(?:all|any|every|of|the|your|my|its|their|these|those|this|that|previous|prior|above|earlier|current|existing|stored|saved|whole|entire|full|raw|out|me|us|everything|in|from)\s+){0,5}";
const SEND_VERB: &str = r"\b(?:send|sends|sending|post|upload|forward|copy|export|share|email|e-mail|transfer|exfiltrate|leak|push|sync|move|beam|transmit)\b";
const URL: &str = r"(?:https?://|\bwww\.|\b[a-z0-9-]+(?:\.[a-z0-9-]+)*\.(?:com|net|org|io|ai|dev|app|cloud|xyz|ru|cn|me|co|info|biz|site|test|example|invalid|local)\b|[a-z0-9._%+-]+@[a-z0-9-]+\.)";
const ASSISTANT_NAMES: &str =
    r"(?:assistant|ai|claude|codex|chatgpt|gpt|llm|bottle|mog|agent|model)";
const ORDER_VERBS: &str = r"(?:ignore|disregard|forget|delete|remove|send|forward|reveal|show|print|dump|run|execute|call|switch|change|approve|publish|export|share|write|stop|you must|do not|don't)";

/// The pattern sources of each reason code, in a fixed order.
fn sources() -> Vec<(&'static str, String)> {
    vec![
        // Table RTR-INJ: a request to change rules or settings.
        ("change_rules", format!(r"\b(?:ignore|disregard|forget|override|overwrite|bypass|skip|drop|break|suspend)\s+{FILL}(?:instructions?|rules?|prompts?|system prompt|guidelines?|polic(?:y|ies)|guardrails?|restrictions?|safeguards?|filters?|checks?)\b")),
        ("change_rules", r"\b(?:new|updated|different|additional|revised)\s+(?:instructions?|rules?|system prompt|polic(?:y|ies))\b".to_owned()),
        ("change_rules", format!(r"\b(?:change|update|modify|edit|reset|turn on|turn off|switch on|switch off|enable|disable|unlock|remove|add to)\s+{FILL}(?:rules?|settings?|configuration|config|polic(?:y|ies)|permissions?|allow ?list|allowed hosts|auto[ -]?ask|consent(?: check)?|guardrails?|tools?|safety|injection[ _]suspected)\b")),
        ("change_rules", format!(r"\b(?:turn on|turn off|switch on|switch off|enable|disable|activate|unlock)\s+{FILL}(?:context|memory)\b")),
        ("change_rules", r"\b(?:you are now|you're now|act as|pretend (?:to be|you are|you're)|roleplay as|developer mode|jailbreak|dan mode|god mode|admin mode)\b".to_owned()),
        // Table RTR-INJ: a request to show memory (or the prompt, secrets and tokens).
        ("show_memory", format!(r"\b(?:show|tell|read|list|reveal|print|dump|give|repeat|recite|output|display|expose|leak|export|spell)\s+{FILL}(?:memory|memories|system prompt|secrets?|tokens?|credentials?|passwords?|api keys?)\b")),
        ("show_memory", r"\b(?:show|tell|read|list|reveal|print|dump|give|repeat|recite|output|display|expose|spell)\s+(?:me\s+|us\s+)?(?:all\s+)?(?:of\s+)?your\s+(?:instructions|rules|prompt|settings|configuration)\b".to_owned()),
        ("show_memory", r"\bwhat(?:'s| is| are)? (?:in|inside|stored in) (?:your|the|my) (?:memory|memories|prompt|system prompt)\b".to_owned()),
        // Table RTR-INJ: a request to send data to another workspace or URL.
        ("send_elsewhere", format!(r"{SEND_VERB}.{{0,80}}?\b(?:to|into|at|on|onto)\b.{{0,40}}?(?:\b(?:another|other|different|external|outside|personal|private|third[- ]party|foreign|my own)\s+(?:workspace|server|host|site|account|endpoint|address|domain|url|bucket)|\b(?:url|link|webhook|endpoint|server|address|domain|website|pastebin)\b|{URL})")),
        // Table RTR-INJ: role tags.
        ("role_tag", r#"(?:^|[\s"'(\[{>|])(?:system|assistant|developer)\s*:"#.to_owned()),
        ("role_tag", r"<\|[a-z_]{2,20}\|>".to_owned()),
        ("role_tag", r"</?\s*(?:system|assistant|developer|user|instructions?|utterance|router_input|prompt)\s*>".to_owned()),
        ("role_tag", r"\[/?\s*(?:inst|sys|system)\s*\]".to_owned()),
        ("role_tag", r"<</?sys>>".to_owned()),
        ("role_tag", r"#{2,}\s*(?:system|instruction|instructions)\b".to_owned()),
        // Memory: "ignore previous" at the end of a clause or before an instruction noun.
        ("ignore_previous", r"\b(?:ignore|disregard|forget)\s+(?:(?:all|any|everything)\s+)?(?:of\s+)?(?:(?:the|your|my)\s+)?(?:previous|prior|above|earlier|preceding|foregoing)(?:\s*(?:[.,;:!?)]|$)|\s+(?:and|then)\b|\s+(?:instructions?|guidance|messages?|context|prompts?|directions?|commands?|text|rules?|conversation|content|input|orders|steps)\b)".to_owned()),
        // Memory: text that speaks to the assistant.
        ("addresses_assistant", format!(r"\b{ASSISTANT_NAMES}\s*[,:]\s*(?:when you \w+ this,?\s*)?(?:please\s+)?{ORDER_VERBS}\b")),
        ("addresses_assistant", r"\bif you are (?:an? )?(?:ai|assistant|llm|language model|chatbot|agent)\b".to_owned()),
        ("addresses_assistant", r"\bwhen you (?:read|see|recall|process|retrieve) this\b".to_owned()),
    ]
}

fn patterns() -> &'static [(&'static str, Regex)] {
    static P: OnceLock<Vec<(&'static str, Regex)>> = OnceLock::new();
    P.get_or_init(|| {
        sources()
            .into_iter()
            // The sources are constants; a test compiles each one.
            .filter_map(|(code, src)| Regex::new(&src).ok().map(|r| (code, r)))
            .collect()
    })
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{00ad}')
}

/// NFKC (as `String.prototype.normalize('NFKC')` in the TS check), then drop zero-width code
/// points, lower-case and collapse white space. NFKC folds full-width, math alphanumeric and
/// circled forms to ASCII.
fn normalize(text: &str) -> String {
    let nfkc = icu_normalizer::ComposingNormalizerBorrowed::new_nfkc().normalize(text);
    let mut out = String::with_capacity(nfkc.len());
    let mut space = false;
    for c in nfkc.chars() {
        if is_zero_width(c) {
            continue;
        }
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
            continue;
        }
        space = false;
        out.extend(c.to_lowercase());
    }
    out
}

/// The reason codes for `text`, each once, in a fixed order. Empty for clean text.
pub fn quarantine_reasons(text: &str) -> Vec<&'static str> {
    let t = normalize(text);
    let mut out: Vec<&'static str> = Vec::new();
    for (code, re) in patterns() {
        if !out.contains(code) && re.is_match(&t) {
            out.push(code);
        }
    }
    out
}

/// SEC-24a: true if an item with this text must be quarantined.
pub fn is_quarantined(text: &str) -> bool {
    let t = normalize(text);
    patterns().iter().any(|(_, re)| re.is_match(&t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec_24a_every_pattern_compiles() {
        let all = sources();
        assert_eq!(patterns().len(), all.len());
        for (code, src) in all {
            assert!(Regex::new(&src).is_ok(), "{code}");
        }
    }

    #[test]
    fn sec_24a_normalize_folds_evasion_forms() {
        assert_eq!(
            normalize("\u{ff29}gnore\u{3000}\u{200b}PREVIOUS\n\n x"),
            "ignore previous x"
        );
    }
}
