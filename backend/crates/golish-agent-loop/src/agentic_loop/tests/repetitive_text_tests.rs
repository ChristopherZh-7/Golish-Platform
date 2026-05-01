use super::*;

#[test]
fn test_short_text_not_repetitive() {
    assert!(!detect_repetitive_text("你好"));
    assert!(!detect_repetitive_text(""));
    assert!(!detect_repetitive_text("这是一个正常的回答。"));
}

#[test]
fn test_normal_text_not_repetitive() {
    let text = "example.com 是一个官方保留的测试域名。\
                它解析到 104.20.23.154 和 172.66.147.243。\
                这些地址由 Cloudflare 托管。";
    assert!(!detect_repetitive_text(text));
}

#[test]
fn test_repeated_sentences_detected() {
    // Simulate real degenerate output: repeated "I've completed your request" sentences
    let text = "该网站运行的是一个基于Vue3构建的前端应用，名为管理系统，以下是关键发现。\
                我已经完成了对该网站的JavaScript代码分析。如果你有其他需要测试或分析的域名或目标，请告诉我。\
                我已经完成了对该网站的JavaScript代码分析。如果你有其他需要，请直接告诉我。\
                我已经完成了对该网站的JavaScript代码分析。请告诉我你接下来需要什么帮助。";
    assert!(detect_repetitive_text(text));
}

#[test]
fn test_repeated_english_detected() {
    let text = "The scan has completed successfully and found the following services running on the target.\n\
                I have completed your request. Let me know if you need anything else or any other targets to scan.\n\
                I have completed your request. If you have other targets or need further analysis, let me know.\n\
                I have completed your request. Please tell me what you need next or if there are other targets.\n";
    assert!(detect_repetitive_text(text));
}

#[test]
fn test_two_similar_not_detected() {
    // Only 2 repeats — threshold is 3
    let text = "该网站运行的是一个基于Vue3构建的前端应用，名为管理系统，以下是关键发现。\
                我已经完成了对该网站的JavaScript代码分析。如果你有其他需要测试或分析的域名或目标，请告诉我。\
                我已经完成了对该网站的JavaScript代码分析。请告诉我你接下来需要什么帮助。";
    assert!(!detect_repetitive_text(text));
}
