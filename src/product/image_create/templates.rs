use super::models::{ImageCreateError, PresetTemplate, TemplateChoice};

const PPT_BUSINESS_ILLUSTRATION_GUIDANCE: &str = "生成适合商业 PPT 的配图。要求：构图简洁、留白充足、主体居中或遵循三分法；风格偏扁平/等距(isometric)/2.5D 插画，色彩克制（主色+1-2 辅色，避免高饱和）；无文字、无水印；背景干净（纯色或轻渐变）；尺寸适配 16:9 或 1:1；视觉传达一个清晰概念，专业克制，不花哨。";
const BUSINESS_FLOW_DIAGRAM_GUIDANCE: &str = "生成清晰的业务流程图/示意图。要求：明确表达步骤/角色/流向，使用箭头、方框、连线等结构化元素；配色用语义色（开始/结束/判断/动作各一色），整体克制；节点文字为英文或简短中文，排版整齐对齐；风格扁平、线条统一粗细；适合插入文档说明，背景干净。";
const WEB_PAGE_UI_GUIDANCE: &str = "生成 Web 页面 UI 设计图。要求：现代干净的网页界面布局，包含明确的导航栏、内容区、侧边栏/卡片等结构化区块；遵循常见设计系统（间距统一、圆角适度、阴影克制）；配色专业（主色+中性灰为主，辅以语义色点缀）；按钮、输入框、表格等控件精致对齐；整体高保真、接近真实产品截图质感；适合作为产品原型或设计稿展示，背景干净。";

pub fn preset_templates() -> Vec<PresetTemplate> {
    vec![
        PresetTemplate::PptBusinessIllustration,
        PresetTemplate::BusinessFlowDiagram,
        PresetTemplate::WebPageUi,
    ]
}

pub fn preset_guidance(template: PresetTemplate) -> &'static str {
    match template {
        PresetTemplate::PptBusinessIllustration => PPT_BUSINESS_ILLUSTRATION_GUIDANCE,
        PresetTemplate::BusinessFlowDiagram => BUSINESS_FLOW_DIAGRAM_GUIDANCE,
        PresetTemplate::WebPageUi => WEB_PAGE_UI_GUIDANCE,
    }
}

pub fn resolve_guidance(choice: &TemplateChoice) -> Result<String, ImageCreateError> {
    if let Some(preset) = choice.preset.clone() {
        return Ok(preset_guidance(preset).to_string());
    }

    if let Some(custom) = choice.custom.as_deref() {
        let trimmed = custom.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    Err(ImageCreateError::InvalidParam(
        "template guidance must select a preset or provide non-empty custom guidance".to_string(),
    ))
}

pub fn build_iteration_prompt(guidance: &str, user_message: &str) -> String {
    format!("{guidance}\n\n用户诉求：{user_message}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_both_preset_templates() {
        assert_eq!(
            preset_templates(),
            vec![
                PresetTemplate::PptBusinessIllustration,
                PresetTemplate::BusinessFlowDiagram,
                PresetTemplate::WebPageUi,
            ]
        );
    }

    #[test]
    fn preset_guidance_matches_prd_8_2_key_phrases_verbatim() {
        let ppt = preset_guidance(PresetTemplate::PptBusinessIllustration);
        assert!(ppt.starts_with("生成适合商业 PPT 的配图。要求：构图简洁、留白充足"));
        assert!(ppt.contains("风格偏扁平/等距(isometric)/2.5D 插画"));

        let flow = preset_guidance(PresetTemplate::BusinessFlowDiagram);
        assert!(flow.starts_with("生成清晰的业务流程图/示意图。要求：明确表达步骤/角色/流向"));
        assert!(flow.contains("使用箭头、方框、连线等结构化元素"));
    }

    #[test]
    fn resolves_preset_guidance() {
        let choice = TemplateChoice {
            preset: Some(PresetTemplate::PptBusinessIllustration),
            custom: None,
        };

        assert_eq!(
            resolve_guidance(&choice).unwrap(),
            preset_guidance(PresetTemplate::PptBusinessIllustration)
        );
    }

    #[test]
    fn resolves_trimmed_non_empty_custom_guidance() {
        let choice = TemplateChoice {
            preset: None,
            custom: Some("  使用品牌蓝色的极简风格  ".to_string()),
        };

        assert_eq!(resolve_guidance(&choice).unwrap(), "使用品牌蓝色的极简风格");
    }

    #[test]
    fn rejects_empty_or_missing_guidance() {
        let empty = TemplateChoice {
            preset: None,
            custom: Some(" \n\t ".to_string()),
        };
        let missing = TemplateChoice {
            preset: None,
            custom: None,
        };

        assert!(matches!(
            resolve_guidance(&empty),
            Err(ImageCreateError::InvalidParam(_))
        ));
        assert!(matches!(
            resolve_guidance(&missing),
            Err(ImageCreateError::InvalidParam(_))
        ));
    }

    #[test]
    fn iteration_prompt_contains_guidance_and_user_message() {
        let prompt = build_iteration_prompt("保持专业、简洁", "画一个产品上线流程");

        assert!(prompt.contains("保持专业、简洁"));
        assert!(prompt.contains("画一个产品上线流程"));
    }
}
