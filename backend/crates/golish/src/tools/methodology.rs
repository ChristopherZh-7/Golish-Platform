use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodologyTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub phases: Vec<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Phase {
    pub id: String,
    pub name: String,
    pub description: String,
    pub items: Vec<CheckItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub checked: bool,
    pub notes: String,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMethodology {
    pub id: String,
    pub template_id: String,
    pub template_name: String,
    pub project_name: String,
    pub phases: Vec<Phase>,
    pub created_at: String,
    pub updated_at: String,
}

fn method_dir(project_path: Option<&str>) -> Result<PathBuf, String> {
    if let Some(pp) = project_path {
        if !pp.is_empty() {
            return Ok(PathBuf::from(pp).join(".golish").join("methodology"));
        }
    }
    let base = dirs::data_dir().ok_or("Cannot resolve data dir")?;
    Ok(base.join("golish-platform").join("methodology"))
}

fn templates_dir() -> Result<PathBuf, String> {
    // Templates are always global (shared across projects)
    let base = dirs::data_dir().ok_or("Cannot resolve data dir")?;
    Ok(base.join("golish-platform").join("methodology").join("templates"))
}

fn projects_dir(project_path: Option<&str>) -> Result<PathBuf, String> {
    Ok(method_dir(project_path)?.join("projects"))
}

fn built_in_templates() -> Vec<MethodologyTemplate> {
    vec![
        MethodologyTemplate {
            id: "owasp-wstg".to_string(),
            name: "OWASP WSTG".to_string(),
            description: "OWASP Web Security Testing Guide - 系统化Web应用安全测试方法论".to_string(),
            phases: vec![
                Phase {
                    id: "info-gathering".to_string(),
                    name: "信息收集 (Information Gathering)".to_string(),
                    description: "收集目标应用的技术架构、入口点和暴露面信息".to_string(),
                    items: vec![
                        check("wstg-info-01", "搜索引擎侦察", "使用搜索引擎发现目标信息泄露", &["subfinder", "amass"]),
                        check("wstg-info-02", "Web服务器指纹", "识别Web服务器类型和版本", &["whatweb", "httpx"]),
                        check("wstg-info-03", "Web应用框架指纹", "识别后端框架和技术栈", &["whatweb"]),
                        check("wstg-info-04", "枚举Web应用入口", "发现应用的所有入口点和参数", &["katana", "ffuf"]),
                        check("wstg-info-05", "网页注释和元数据", "检查HTML注释和元数据泄露", &[]),
                        check("wstg-info-06", "应用入口点识别", "映射所有HTTP端点和参数", &["katana"]),
                        check("wstg-info-07", "映射执行路径", "理解应用的执行流程", &[]),
                        check("wstg-info-08", "指纹Web应用框架", "深入分析框架特征", &["whatweb"]),
                        check("wstg-info-09", "映射应用架构", "了解网络拓扑和基础设施", &["nmap"]),
                    ],
                },
                Phase {
                    id: "config-mgmt".to_string(),
                    name: "配置管理测试 (Configuration)".to_string(),
                    description: "测试应用和基础设施配置中的安全弱点".to_string(),
                    items: vec![
                        check("wstg-conf-01", "网络基础设施配置", "测试网络层安全配置", &["nmap"]),
                        check("wstg-conf-02", "应用平台配置", "审查应用服务器配置", &["nikto"]),
                        check("wstg-conf-03", "文件扩展名处理", "测试敏感文件扩展名处理", &["ffuf"]),
                        check("wstg-conf-04", "备份文件发现", "搜索旧备份和临时文件", &["ffuf", "gobuster"]),
                        check("wstg-conf-05", "枚举管理接口", "发现管理后台和接口", &["ffuf", "gobuster"]),
                        check("wstg-conf-06", "HTTP方法测试", "测试允许的HTTP方法", &["httpx"]),
                        check("wstg-conf-07", "HTTP严格传输安全", "验证HSTS配置", &["httpx"]),
                        check("wstg-conf-08", "跨域策略", "审查CORS和跨域配置", &[]),
                        check("wstg-conf-09", "文件权限测试", "检查敏感文件权限", &[]),
                        check("wstg-conf-10", "子域名枚举", "发现所有相关子域名", &["subfinder", "amass", "dnsx"]),
                    ],
                },
                Phase {
                    id: "identity-mgmt".to_string(),
                    name: "身份认证测试 (Identity)".to_string(),
                    description: "测试身份验证和会话管理安全性".to_string(),
                    items: vec![
                        check("wstg-idnt-01", "角色定义测试", "审查用户角色和权限定义", &[]),
                        check("wstg-idnt-02", "用户注册流程", "测试注册过程中的安全问题", &[]),
                        check("wstg-idnt-03", "账户配置流程", "审查账户创建和配置", &[]),
                        check("wstg-idnt-04", "用户名枚举", "测试是否可枚举有效用户名", &[]),
                        check("wstg-authn-01", "传输层加密", "验证认证凭据传输安全", &[]),
                        check("wstg-authn-02", "默认凭据测试", "测试默认账号密码", &[]),
                        check("wstg-authn-03", "锁定机制", "测试账户锁定和暴力破解保护", &[]),
                        check("wstg-authn-04", "认证绕过测试", "尝试绕过认证机制", &[]),
                        check("wstg-authn-05", "密码找回测试", "测试密码重置流程安全", &[]),
                    ],
                },
                Phase {
                    id: "injection".to_string(),
                    name: "注入测试 (Injection)".to_string(),
                    description: "测试各类注入漏洞".to_string(),
                    items: vec![
                        check("wstg-inpv-01", "反射型XSS", "测试反射型跨站脚本攻击", &["XSStrike", "nuclei"]),
                        check("wstg-inpv-02", "存储型XSS", "测试存储型跨站脚本攻击", &["XSStrike"]),
                        check("wstg-inpv-03", "HTTP参数篡改", "测试HTTP参数操纵", &[]),
                        check("wstg-inpv-05", "SQL注入", "测试SQL注入漏洞", &["nuclei"]),
                        check("wstg-inpv-06", "LDAP注入", "测试LDAP注入漏洞", &[]),
                        check("wstg-inpv-07", "XML注入", "测试XML注入和XXE", &["nuclei"]),
                        check("wstg-inpv-08", "SSI注入", "测试服务端包含注入", &[]),
                        check("wstg-inpv-09", "XPath注入", "测试XPath注入", &[]),
                        check("wstg-inpv-11", "代码注入", "测试服务端代码注入", &["nuclei"]),
                        check("wstg-inpv-12", "命令注入", "测试操作系统命令注入", &["nuclei"]),
                        check("wstg-inpv-13", "模板注入", "测试服务端模板注入(SSTI)", &["nuclei"]),
                        check("wstg-inpv-14", "SSRF", "测试服务器端请求伪造", &["nuclei"]),
                    ],
                },
                Phase {
                    id: "business-logic".to_string(),
                    name: "业务逻辑测试 (Business Logic)".to_string(),
                    description: "测试应用特定的业务逻辑缺陷".to_string(),
                    items: vec![
                        check("wstg-busl-01", "数据验证测试", "测试输入验证和数据完整性", &[]),
                        check("wstg-busl-02", "请求伪造", "测试请求参数可否被篡改", &[]),
                        check("wstg-busl-03", "完整性检查", "验证应用的完整性校验机制", &[]),
                        check("wstg-busl-04", "时序测试", "测试竞态条件和时序攻击", &[]),
                        check("wstg-busl-05", "使用次数限制", "测试功能使用次数限制", &[]),
                        check("wstg-busl-06", "工作流绕过", "测试工作流程是否可被绕过", &[]),
                        check("wstg-busl-07", "应用误用", "测试应用防御异常使用的能力", &[]),
                        check("wstg-busl-08", "文件上传测试", "测试文件上传功能安全", &[]),
                    ],
                },
            ],
        },
        MethodologyTemplate {
            id: "ptes".to_string(),
            name: "PTES".to_string(),
            description: "Penetration Testing Execution Standard - 渗透测试执行标准".to_string(),
            phases: vec![
                Phase {
                    id: "ptes-intel".to_string(),
                    name: "情报收集 (Intelligence Gathering)".to_string(),
                    description: "被动和主动信息收集".to_string(),
                    items: vec![
                        check("ptes-intel-01", "OSINT收集", "开源情报收集和分析", &["subfinder", "amass"]),
                        check("ptes-intel-02", "DNS侦察", "DNS记录查询和区域传输", &["dnsx", "subfinder"]),
                        check("ptes-intel-03", "端口扫描", "TCP/UDP端口扫描", &["nmap", "rustscan", "masscan"]),
                        check("ptes-intel-04", "服务枚举", "识别运行服务和版本", &["nmap"]),
                        check("ptes-intel-05", "操作系统指纹", "远程OS检测", &["nmap"]),
                        check("ptes-intel-06", "Web应用侦察", "发现Web应用入口", &["httpx", "katana", "whatweb"]),
                    ],
                },
                Phase {
                    id: "ptes-vuln".to_string(),
                    name: "漏洞分析 (Vulnerability Analysis)".to_string(),
                    description: "漏洞识别和验证".to_string(),
                    items: vec![
                        check("ptes-vuln-01", "自动化扫描", "使用扫描器进行漏洞检测", &["nuclei", "nikto"]),
                        check("ptes-vuln-02", "手动验证", "手动验证扫描器发现的漏洞", &[]),
                        check("ptes-vuln-03", "CVE研究", "查询已知CVE和公开exploit", &[]),
                        check("ptes-vuln-04", "配置审计", "审查安全配置", &[]),
                    ],
                },
                Phase {
                    id: "ptes-exploit".to_string(),
                    name: "漏洞利用 (Exploitation)".to_string(),
                    description: "尝试利用已发现的漏洞".to_string(),
                    items: vec![
                        check("ptes-exploit-01", "已知漏洞利用", "使用公开exploit验证漏洞", &["metasploit"]),
                        check("ptes-exploit-02", "密码攻击", "暴力破解和字典攻击", &["john"]),
                        check("ptes-exploit-03", "Web应用攻击", "利用Web漏洞获取访问", &["XSStrike"]),
                        check("ptes-exploit-04", "网络攻击", "网络层攻击和中间人", &["chisel"]),
                    ],
                },
                Phase {
                    id: "ptes-post".to_string(),
                    name: "后渗透 (Post-Exploitation)".to_string(),
                    description: "获取初始访问后的后续操作".to_string(),
                    items: vec![
                        check("ptes-post-01", "权限提升", "尝试提升系统权限", &[]),
                        check("ptes-post-02", "持久化", "建立持久访问通道", &["chisel"]),
                        check("ptes-post-03", "数据收集", "收集敏感数据和凭据", &[]),
                        check("ptes-post-04", "横向移动", "在网络中横向渗透", &[]),
                        check("ptes-post-05", "痕迹清理", "清理测试痕迹", &[]),
                    ],
                },
                Phase {
                    id: "ptes-report".to_string(),
                    name: "报告 (Reporting)".to_string(),
                    description: "编写测试报告".to_string(),
                    items: vec![
                        check("ptes-report-01", "执行摘要", "编写管理层执行摘要", &[]),
                        check("ptes-report-02", "技术发现", "详细记录每个发现", &[]),
                        check("ptes-report-03", "风险评级", "为每个发现分配风险等级", &[]),
                        check("ptes-report-04", "修复建议", "提供具体修复方案", &[]),
                    ],
                },
            ],
        },
    ]
}

fn check(id: &str, title: &str, desc: &str, tools: &[&str]) -> CheckItem {
    CheckItem {
        id: id.to_string(),
        title: title.to_string(),
        description: desc.to_string(),
        checked: false,
        notes: String::new(),
        tools: tools.iter().map(|s| s.to_string()).collect(),
    }
}

#[tauri::command]
pub async fn method_list_templates() -> Result<Vec<MethodologyTemplate>, String> {
    let mut templates = built_in_templates();

    let custom_dir = templates_dir()?;
    if custom_dir.exists() {
        let mut entries = fs::read_dir(&custom_dir).await.map_err(|e| e.to_string())?;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            if entry.path().extension().map_or(false, |e| e == "json") {
                if let Ok(content) = fs::read_to_string(entry.path()).await {
                    if let Ok(t) = serde_json::from_str::<MethodologyTemplate>(&content) {
                        templates.push(t);
                    }
                }
            }
        }
    }

    Ok(templates)
}

#[tauri::command]
pub async fn method_start_project(
    template_id: String,
    project_name: String,
    project_path: Option<String>,
) -> Result<ProjectMethodology, String> {
    let templates = built_in_templates();
    let template = templates
        .iter()
        .find(|t| t.id == template_id)
        .ok_or("Template not found")?;

    let now = chrono::Utc::now().to_rfc3339();
    let project = ProjectMethodology {
        id: Uuid::new_v4().to_string(),
        template_id: template.id.clone(),
        template_name: template.name.clone(),
        project_name,
        phases: template.phases.clone(),
        created_at: now.clone(),
        updated_at: now,
    };

    let dir = projects_dir(project_path.as_deref())?;
    fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", project.id));
    let json = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    fs::write(&path, json).await.map_err(|e| e.to_string())?;

    Ok(project)
}

#[tauri::command]
pub async fn method_list_projects(project_path: Option<String>) -> Result<Vec<ProjectMethodology>, String> {
    let dir = projects_dir(project_path.as_deref())?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut projects = Vec::new();
    let mut entries = fs::read_dir(&dir).await.map_err(|e| e.to_string())?;
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        if entry.path().extension().map_or(false, |e| e == "json") {
            if let Ok(content) = fs::read_to_string(entry.path()).await {
                if let Ok(p) = serde_json::from_str::<ProjectMethodology>(&content) {
                    projects.push(p);
                }
            }
        }
    }
    projects.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(projects)
}

#[tauri::command]
pub async fn method_load_project(id: String, project_path: Option<String>) -> Result<ProjectMethodology, String> {
    let dir = projects_dir(project_path.as_deref())?;
    let path = dir.join(format!("{}.json", id));
    let content = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    serde_json::from_str(&content).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn method_update_item(
    project_id: String,
    phase_id: String,
    item_id: String,
    checked: Option<bool>,
    notes: Option<String>,
    project_path: Option<String>,
) -> Result<(), String> {
    let dir = projects_dir(project_path.as_deref())?;
    let path = dir.join(format!("{}.json", project_id));
    let content = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    let mut project: ProjectMethodology = serde_json::from_str(&content).map_err(|e| e.to_string())?;

    for phase in &mut project.phases {
        if phase.id == phase_id {
            for item in &mut phase.items {
                if item.id == item_id {
                    if let Some(c) = checked {
                        item.checked = c;
                    }
                    if let Some(ref n) = notes {
                        item.notes = n.clone();
                    }
                }
            }
        }
    }

    project.updated_at = chrono::Utc::now().to_rfc3339();
    let json = serde_json::to_string_pretty(&project).map_err(|e| e.to_string())?;
    fs::write(&path, json).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn method_delete_project(id: String, project_path: Option<String>) -> Result<(), String> {
    let dir = projects_dir(project_path.as_deref())?;
    let path = dir.join(format!("{}.json", id));
    if path.exists() {
        fs::remove_file(&path).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}
