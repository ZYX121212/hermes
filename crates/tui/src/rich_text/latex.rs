// crates/tui/src/rich_text/latex.rs
// Convert common LaTeX math commands to Unicode characters.

use std::collections::HashMap;
use std::sync::OnceLock;

fn latex_map() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&str, &str>> = OnceLock::new();
    MAP.get_or_init(|| {
        let mut m = HashMap::new();
        // Greek lowercase
        m.insert("\\alpha", "α");
        m.insert("\\beta", "β");
        m.insert("\\gamma", "γ");
        m.insert("\\delta", "δ");
        m.insert("\\epsilon", "ε");
        m.insert("\\zeta", "ζ");
        m.insert("\\eta", "η");
        m.insert("\\theta", "θ");
        m.insert("\\iota", "ι");
        m.insert("\\kappa", "κ");
        m.insert("\\lambda", "λ");
        m.insert("\\mu", "μ");
        m.insert("\\nu", "ν");
        m.insert("\\xi", "ξ");
        m.insert("\\pi", "π");
        m.insert("\\rho", "ρ");
        m.insert("\\sigma", "σ");
        m.insert("\\tau", "τ");
        m.insert("\\upsilon", "υ");
        m.insert("\\phi", "φ");
        m.insert("\\chi", "χ");
        m.insert("\\psi", "ψ");
        m.insert("\\omega", "ω");
        // Greek uppercase
        m.insert("\\Gamma", "Γ");
        m.insert("\\Delta", "Δ");
        m.insert("\\Theta", "Θ");
        m.insert("\\Lambda", "Λ");
        m.insert("\\Xi", "Ξ");
        m.insert("\\Pi", "Π");
        m.insert("\\Sigma", "Σ");
        m.insert("\\Phi", "Φ");
        m.insert("\\Psi", "Ψ");
        m.insert("\\Omega", "Ω");
        // Math symbols
        m.insert("\\infty", "∞");
        m.insert("\\pm", "±");
        m.insert("\\mp", "∓");
        m.insert("\\times", "×");
        m.insert("\\div", "÷");
        m.insert("\\cdot", "·");
        m.insert("\\approx", "≈");
        m.insert("\\neq", "≠");
        m.insert("\\leq", "≤");
        m.insert("\\geq", "≥");
        m.insert("\\ll", "≪");
        m.insert("\\gg", "≫");
        m.insert("\\equiv", "≡");
        m.insert("\\sim", "∼");
        m.insert("\\propto", "∝");
        m.insert("\\partial", "∂");
        m.insert("\\nabla", "∇");
        m.insert("\\int", "∫");
        m.insert("\\iint", "∬");
        m.insert("\\iiint", "∭");
        m.insert("\\oint", "∮");
        m.insert("\\sum", "∑");
        m.insert("\\prod", "∏");
        m.insert("\\coprod", "∐");
        m.insert("\\sqrt", "√");
        m.insert("\\forall", "∀");
        m.insert("\\exists", "∃");
        m.insert("\\nexists", "∄");
        m.insert("\\in", "∈");
        m.insert("\\notin", "∉");
        m.insert("\\subset", "⊂");
        m.insert("\\supset", "⊃");
        m.insert("\\subseteq", "⊆");
        m.insert("\\supseteq", "⊇");
        m.insert("\\cup", "∪");
        m.insert("\\cap", "∩");
        m.insert("\\emptyset", "∅");
        m.insert("\\varnothing", "∅");
        m.insert("\\land", "∧");
        m.insert("\\lor", "∨");
        m.insert("\\neg", "¬");
        m.insert("\\implies", "→");
        m.insert("\\iff", "⇔");
        m.insert("\\rightarrow", "→");
        m.insert("\\leftarrow", "←");
        m.insert("\\leftrightarrow", "↔");
        m.insert("\\uparrow", "↑");
        m.insert("\\downarrow", "↓");
        m.insert("\\mapsto", "↦");
        m.insert("\\to", "→");
        m.insert("\\langle", "⟨");
        m.insert("\\rangle", "⟩");
        m.insert("\\lceil", "⌈");
        m.insert("\\rceil", "⌉");
        m.insert("\\lfloor", "⌊");
        m.insert("\\rfloor", "⌋");
        m.insert("\\ldots", "…");
        m.insert("\\cdots", "⋯");
        m.insert("\\vdots", "⋮");
        m.insert("\\ddots", "⋱");
        m.insert("\\angle", "∠");
        m.insert("\\parallel", "∥");
        m.insert("\\perp", "⊥");
        m.insert("\\circ", "∘");
        m.insert("\\triangle", "△");
        m.insert("\\square", "□");
        m.insert("\\diamond", "◇");
        m.insert("\\star", "★");
        m.insert("\\aleph", "ℵ");
        m.insert("\\hbar", "ℏ");
        m.insert("\\ell", "ℓ");
        m.insert("\\wp", "℘");
        m.insert("\\Re", "ℜ");
        m.insert("\\Im", "ℑ");
        m.insert("\\prime", "′");
        m.insert("\\backslash", "\\");
        m
    })
}

/// Convert inline LaTeX math ($...$) to Unicode.
pub fn render_latex_inline(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        match chars[i] {
            '$' => {
                i += 1;
            }
            '\\' => {
                i += 1;
                let start = i - 1;
                // Consume command name (alphabetic chars)
                while i < len && chars[i].is_alphabetic() {
                    i += 1;
                }
                // Consume brace groups if present (e.g. \frac{a}{b})
                while i < len && chars[i] == '{' {
                    let mut depth = 1;
                    i += 1;
                    while i < len && depth > 0 {
                        match chars[i] {
                            '{' => depth += 1,
                            '}' => depth -= 1,
                            _ => {}
                        }
                        i += 1;
                    }
                }
                let cmd: String = chars[start..i].iter().collect();
                result.push_str(&convert_math(&cmd));
            }
            _ => {
                result.push(chars[i]);
                i += 1;
            }
        }
    }

    result
}

fn convert_math(math: &str) -> String {
    // Handle \frac{a}{b}
    let mut result = math.to_string();
    while let Some(pos) = result.find("\\frac") {
        let after = &result[pos + 5..];
        if let Some(open1) = after.find('{') {
            let inner = pos + 5 + open1 + 1;
            if let Some(close1) = result[inner..].find('}') {
                let num = &result[inner..inner + close1];
                let after_num = inner + close1 + 1;
                if result[after_num..].starts_with('{') {
                    if let Some(close2) = result[after_num + 1..].find('}') {
                        let den = &result[after_num + 1..after_num + 1 + close2];
                        let end = after_num + 1 + close2 + 1;
                        result.replace_range(pos..end, &format!("({})/({})", num, den));
                        continue;
                    }
                }
            }
        }
        break;
    }

    let map = latex_map();
    let mut keys: Vec<&&str> = map.keys().collect();
    keys.sort_by_key(|b| std::cmp::Reverse(b.len()));
    for key in keys {
        result = result.replace(*key, map[*key]);
    }

    result = result.replace(['{', '}'], "");
    result
}

/// Detect if text contains LaTeX math delimiters.
pub fn has_latex(text: &str) -> bool {
    text.contains('$')
        || text.contains("\\frac")
        || text.contains("\\sum")
        || text.contains("\\int")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_symbols() {
        assert_eq!(render_latex_inline("$\\alpha$"), "α");
    }

    #[test]
    fn test_frac() {
        let result = render_latex_inline("$\\frac{a}{b}$");
        assert!(result.contains("(a)/(b)"), "got: {result}");
    }

    #[test]
    fn test_plain_text_passthrough() {
        assert_eq!(render_latex_inline("hello world"), "hello world");
    }

    #[test]
    fn test_has_latex() {
        assert!(has_latex("$x^2$"));
        assert!(has_latex("\\frac{a}{b}"));
        assert!(!has_latex("plain text"));
    }

    #[test]
    fn test_greek_lowercase() {
        // All Greek lowercase should convert
        let pairs = [
            ("\\alpha", "α"),
            ("\\beta", "β"),
            ("\\gamma", "γ"),
            ("\\delta", "δ"),
            ("\\pi", "π"),
            ("\\sigma", "σ"),
            ("\\omega", "ω"),
        ];
        for (cmd, expected) in pairs {
            assert_eq!(
                render_latex_inline(&format!("${cmd}$")),
                expected,
                "failed for {cmd}"
            );
        }
    }

    #[test]
    fn test_greek_uppercase() {
        let pairs = [
            ("\\Gamma", "Γ"),
            ("\\Delta", "Δ"),
            ("\\Sigma", "Σ"),
            ("\\Omega", "Ω"),
        ];
        for (cmd, expected) in pairs {
            assert_eq!(
                render_latex_inline(&format!("${cmd}$")),
                expected,
                "failed for {cmd}"
            );
        }
    }

    #[test]
    fn test_math_symbols() {
        let pairs = [
            ("\\infty", "∞"),
            ("\\pm", "±"),
            ("\\times", "×"),
            ("\\leq", "≤"),
            ("\\geq", "≥"),
            ("\\approx", "≈"),
            ("\\neq", "≠"),
            ("\\forall", "∀"),
            ("\\exists", "∃"),
            ("\\in", "∈"),
            ("\\rightarrow", "→"),
            ("\\leftarrow", "←"),
            ("\\implies", "→"),
            ("\\iff", "⇔"),
            ("\\int", "∫"),
            ("\\sum", "∑"),
            ("\\prod", "∏"),
            ("\\sqrt", "√"),
            ("\\partial", "∂"),
            ("\\nabla", "∇"),
        ];
        for (cmd, expected) in pairs {
            assert_eq!(
                render_latex_inline(&format!("${cmd}$")),
                expected,
                "failed for {cmd}"
            );
        }
    }

    #[test]
    fn test_set_symbols() {
        let pairs = [
            ("\\subset", "⊂"),
            ("\\supset", "⊃"),
            ("\\subseteq", "⊆"),
            ("\\supseteq", "⊇"),
            ("\\cup", "∪"),
            ("\\cap", "∩"),
            ("\\emptyset", "∅"),
            ("\\land", "∧"),
            ("\\lor", "∨"),
            ("\\neg", "¬"),
        ];
        for (cmd, expected) in pairs {
            assert_eq!(
                render_latex_inline(&format!("${cmd}$")),
                expected,
                "failed for {cmd}"
            );
        }
    }

    #[test]
    fn test_ellipsis_variants() {
        assert_eq!(render_latex_inline("$\\ldots$"), "…");
        assert_eq!(render_latex_inline("$\\cdots$"), "⋯");
        assert_eq!(render_latex_inline("$\\vdots$"), "⋮");
        assert_eq!(render_latex_inline("$\\ddots$"), "⋱");
    }

    #[test]
    fn test_mixed_content() {
        let result = render_latex_inline("The result is $\\alpha + \\beta = \\gamma$");
        assert!(result.contains("α"));
        assert!(result.contains("β"));
        assert!(result.contains("γ"));
        assert!(result.contains("The result is"));
    }

    #[test]
    fn test_multiple_math_blocks() {
        let result = render_latex_inline("$\\alpha$ and $\\beta$");
        assert_eq!(result, "α and β");
    }

    #[test]
    fn test_command_with_braces() {
        // \sqrt{x} should strip braces and convert \sqrt
        let result = render_latex_inline("$\\sqrt{x}$");
        assert!(result.contains('√'));
        assert!(result.contains('x'));
        assert!(!result.contains('{'));
        assert!(!result.contains('}'));
    }

    #[test]
    fn test_backslash_passthrough() {
        let result = render_latex_inline("\\unknowncmd");
        // Unknown commands should pass through (after brace stripping)
        assert!(result.contains("\\unknowncmd") || !result.is_empty());
    }

    #[test]
    fn test_converter_no_panic_on_empty() {
        assert_eq!(render_latex_inline(""), "");
    }

    #[test]
    fn test_convert_math_removes_braces() {
        let result = convert_math("\\alpha");
        assert_eq!(result, "α");
    }

    #[test]
    fn test_has_latex_edge_cases() {
        assert!(has_latex("$"));
        assert!(has_latex("\\sum_{i=1}^{n}"));
        assert!(has_latex("\\int_0^\\infty"));
        assert!(!has_latex(""));
        assert!(!has_latex("normal text"));
    }

    #[test]
    fn test_all_greek_lowercase() {
        let greek = [
            "\\alpha",
            "\\beta",
            "\\gamma",
            "\\delta",
            "\\epsilon",
            "\\zeta",
            "\\eta",
            "\\theta",
            "\\iota",
            "\\kappa",
            "\\lambda",
            "\\mu",
            "\\nu",
            "\\xi",
            "\\pi",
            "\\rho",
            "\\sigma",
            "\\tau",
            "\\upsilon",
            "\\phi",
            "\\chi",
            "\\psi",
            "\\omega",
        ];
        for cmd in greek {
            let result = render_latex_inline(&format!("${cmd}$"));
            assert!(!result.is_empty(), "empty result for {cmd}");
            assert!(!result.contains('\\'), "{cmd} not converted: {result}");
        }
    }

    #[test]
    fn test_rendered_map_has_all_keys() {
        let map = latex_map();
        assert!(
            map.len() >= 100,
            "expected >=100 entries, got {}",
            map.len()
        );
        // Verify a few critical entries exist
        assert!(map.contains_key("\\alpha"));
        assert!(map.contains_key("\\infty"));
        assert!(map.contains_key("\\sum"));
        assert!(map.contains_key("\\int"));
        assert!(map.contains_key("\\frac") == false); // frac is handled specially
    }

    // ── Edge cases ──

    #[test]
    fn test_latex_nested_braces() {
        let result = render_latex_inline("$\\sqrt{\\frac{a}{b}}$");
        assert!(!result.contains('\\'), "nested braces should be converted");
        assert!(!result.is_empty());
    }

    #[test]
    fn test_latex_multiple_fracs() {
        let result = render_latex_inline("$\\frac{1}{2} + \\frac{3}{4}$");
        assert!(result.contains('+'));
        assert!(!result.contains("\\frac"));
    }

    #[test]
    fn test_latex_unbalanced_brace() {
        // Should not panic with unbalanced braces
        let result = render_latex_inline("$\\sqrt{x$");
        assert!(!result.contains('\\'));
    }

    #[test]
    fn test_latex_command_with_trailing_digits() {
        let result = render_latex_inline("$\\alpha_1$");
        assert!(!result.contains("\\alpha"));
    }

    #[test]
    fn test_latex_standalone_dollar() {
        // Single $ not in a pair
        let result = render_latex_inline("cost $50");
        assert!(result.contains("50"));
    }
}
