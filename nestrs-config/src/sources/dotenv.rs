//! 受限 dotenv 文本解析与环境路径映射。
//!
//! 逐个物理行解析声明，只产生配置层，不写回进程环境。这里刻意不调用 shell，
//! 不展开变量，也不把第三方 dotenv 解析器可能附带的插值行为作为隐式配置能力。

use std::collections::HashSet;
use std::path::PathBuf;

use super::environment::{mapped_path, track_path, validate_options};
use super::file::FileInput;
use crate::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigPath, ConfigSource, LayerValue, LoadContext,
    MAX_NODES, Origin,
};

/// 只读取单行赋值的 dotenv 文件来源。
///
/// 支持空行、注释、可选的 `export` 前缀、单引号字面量和双引号的有限转义。
/// 不支持变量展开、命令执行、多行引号或反斜杠续行；解析后的值保持待转换文本。
/// 该来源不会读取其他环境变量来替换内容，也不会修改进程环境。
#[derive(Clone, Debug)]
pub struct DotEnv {
    input: FileInput,
    prefix: String,
    separator: String,
}

impl DotEnv {
    /// 保存 dotenv 文件路径；默认选择全部键，并以 `__` 分隔对象路径。
    ///
    /// 文件在构建器加载来源时读取，相对路径按本次固定基础目录解析。
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            input: FileInput::new(path),
            prefix: String::new(),
            separator: "__".into(),
        }
    }

    /// 仅忽略文件不存在，语法、编码、权限及非法选项仍会产生错误。
    pub fn optional(mut self) -> Self {
        self.input.optional = true;
        self
    }

    /// 设置区分大小写的键前缀，只把匹配声明映射到配置树。
    ///
    /// 文件中不匹配的声明仍须通过基本语法和重复键检查，不会因筛选而掩盖坏文件。
    pub fn prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = prefix.into();
        self
    }

    /// 设置键路径分隔符；默认 `__`，空分隔符在加载时被拒绝。
    ///
    /// 与环境来源一样，路径段只进行 ASCII 小写转换，数字段仍是对象键。
    pub fn separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }
}

impl ConfigSource for DotEnv {
    /// 逐行校验完整文件，将匹配的文本值及其实际行列加入独立层。
    fn load(&self, context: &LoadContext) -> Result<ConfigLayer, ConfigError> {
        // 即使可选文件缺失，非法选项也必须被报告，不能因环境差异被悄悄隐藏。
        validate_options(&self.separator, &Origin::file(self.input.path.clone()))?;
        let (text, origin) = self.input.read(context)?;
        let Some(text) = text else {
            return Ok(ConfigLayer::empty());
        };
        let mut layer = ConfigLayer::builder(origin.clone());
        // keys 检查原始声明重复；nodes 统计映射后的实际节点。
        // ASCII 小写导致的撞名以及父子冲突由后续层插入继续检查。
        let mut keys = HashSet::new();
        let mut nodes = HashSet::from([ConfigPath::root()]);
        for (line_index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let located = origin.clone().at(line_index + 1, 1)?;
            let declaration = if let Some(after_export) = trimmed.strip_prefix("export") {
                if after_export.starts_with([' ', '\t']) {
                    after_export.trim_start()
                } else {
                    trimmed
                }
            } else {
                trimmed
            };
            let (key, input) = declaration
                .split_once('=')
                .ok_or_else(|| parse_error(&located))?;
            let key = key.trim();
            // 第一版变量名限定为 ASCII 字母或下划线开头，后续可含数字。
            // 不接受 shell 赋值表达式或其他额外语法。
            let mut bytes = key.bytes();
            if !bytes
                .next()
                .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
                || !bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
            {
                return Err(parse_error(&located));
            }
            if !keys.insert(key.to_owned()) {
                return Err(ConfigError::new(ConfigErrorKind::DuplicateKey).with_origin(located));
            }
            if keys.len() >= MAX_NODES {
                return Err(ConfigError::new(ConfigErrorKind::Limit).with_origin(located));
            }
            // 先校验每条声明，再按前缀筛选；文件不能因为坏行未被选中就部分成功。
            let value = parse_value(input.trim_start(), &located)?;
            if !key.starts_with(&self.prefix) {
                continue;
            }
            // input 仍借用当前物理行，地址差加上被跳过的前导空白可得到值的字节列。
            // 保存真实位置，不把整个赋值行或原始值附加到错误消息。
            let column = (input.as_ptr() as usize) - (line.as_ptr() as usize) + input.len()
                - input.trim_start().len()
                + 1;
            let located = origin.clone().at(line_index + 1, column)?;
            let path = mapped_path(key, &self.prefix, &self.separator, &located)?;
            track_path(&mut nodes, &path, &located)?;
            layer
                .insert(path, LayerValue::text(value).with_origin(located.clone()))
                .map_err(|error| error.with_origin(located))?;
        }
        layer.build()
    }
}

/// 构造仅含安全类别及已知来源的语法错误，不携带出错行或值片段。
fn parse_error(origin: &Origin) -> ConfigError {
    ConfigError::new(ConfigErrorKind::Parse).with_origin(origin.clone())
}

/// 解析一个物理行中的值，输入已去掉赋值符之后的前导空白。
///
/// 无引号值仅在行首或空白后的 `#` 处开始注释；单引号内容保持字面语义。
/// 双引号只解释明确列出的转义，`$VAR` 与 `$(...)` 始终是普通文本。
fn parse_value(input: &str, origin: &Origin) -> Result<String, ConfigError> {
    let Some(quote @ ('\'' | '"')) = input.chars().next() else {
        let mut end = input.len();
        let mut previous_whitespace = true;
        for (index, character) in input.char_indices() {
            if character == '#' && previous_whitespace {
                end = index;
                break;
            }
            previous_whitespace = character.is_whitespace();
        }
        let value = input[..end].trim_end();
        // 不拼接下一行。明确拒绝续行样式，避免应用误以为遵循 shell 的续行语义。
        if value.ends_with('\\') {
            return Err(parse_error(origin));
        }
        return Ok(value.to_owned());
    };
    let mut output = String::new();
    let mut characters = input.char_indices();
    characters.next();
    while let Some((index, character)) = characters.next() {
        if character == quote {
            // 闭合引号后只允许空白或注释，不能把额外文本默认为字符串连接。
            let trailing = input[index + character.len_utf8()..].trim_start();
            if !trailing.is_empty() && !trailing.starts_with('#') {
                return Err(parse_error(origin));
            }
            return Ok(output);
        }
        if quote == '"' && character == '\\' {
            // 转义集合保持固定；未知转义直接失败，不删除反斜杠后猜测用户意图。
            let (_, escaped) = characters.next().ok_or_else(|| parse_error(origin))?;
            output.push(match escaped {
                '\\' => '\\',
                '"' => '"',
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                _ => return Err(parse_error(origin)),
            });
        } else {
            output.push(character);
        }
    }
    Err(parse_error(origin))
}
