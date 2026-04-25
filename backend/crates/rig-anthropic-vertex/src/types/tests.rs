use super::*;

#[test]
fn test_image_content_block_serialization() {
    let img = ContentBlock::Image {
        source: ImageSource {
            source_type: "base64".to_string(),
            media_type: "image/png".to_string(),
            data: "iVBORw0KGgoAAAANSUhEUg==".to_string(),
        },
        cache_control: None,
    };

    let json = serde_json::to_string(&img).unwrap();
    println!("Image ContentBlock JSON: {}", json);

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["type"], "image");
    assert_eq!(parsed["source"]["type"], "base64");
    assert_eq!(parsed["source"]["media_type"], "image/png");
    assert!(parsed["source"]["data"].as_str().is_some());
}

#[test]
fn test_message_with_image_serialization() {
    let msg = Message {
        role: Role::User,
        content: vec![
            ContentBlock::Text {
                text: "What is in this image?".to_string(),
                cache_control: None,
            },
            ContentBlock::Image {
                source: ImageSource {
                    source_type: "base64".to_string(),
                    media_type: "image/jpeg".to_string(),
                    data: "base64data".to_string(),
                },
                cache_control: None,
            },
        ],
    };

    let json = serde_json::to_string_pretty(&msg).unwrap();
    println!("Message with image JSON:\n{}", json);

    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["role"], "user");
    assert!(parsed["content"].is_array());
    assert_eq!(parsed["content"][0]["type"], "text");
    assert_eq!(parsed["content"][1]["type"], "image");
    assert_eq!(parsed["content"][1]["source"]["type"], "base64");
}

#[test]
fn test_citations_delta_deserialization() {
    let json = r#"{"type": "citations_delta", "citation": {"type": "web_search_result_location", "cited_text": "Bestia in Los Angeles is ranked second on Yelp's list of best 100 U.S. pizza spots", "url": "https://www.yahoo.com/lifestyle/articles/most-popular-pizzas-los-angeles-090000864.html", "title": "The Most Popular Pizzas At Los Angeles' Top-Rated Spot", "encrypted_index": "EnkKFAgMEAIYAiIMTI2MTI2MTI2MTI2"}}"#;

    let delta: ContentDelta = serde_json::from_str(json).unwrap();

    if let ContentDelta::CitationsDelta { citation } = delta {
        let Citation::WebSearchResultLocation {
            cited_text,
            url,
            title,
            encrypted_index,
        } = citation;
        assert!(cited_text.contains("Bestia"));
        assert!(url.contains("yahoo.com"));
        assert!(title.contains("Popular Pizzas"));
        assert!(!encrypted_index.is_empty());
    } else {
        panic!("Expected CitationsDelta");
    }
}
