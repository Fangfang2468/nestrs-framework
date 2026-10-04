//! 文件来源共用的读取限制与字节位置换算。
//!
//! 先确认普通文件，再限制实际读取字节数并检查 UTF-8；所有错误只附带安全类别及来源。
//! 本模块不负责具体格式解析，也不将文件内容或系统错误原文保存到公开错误中。

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use crate::{ConfigError, ConfigErrorKind, LoadContext, MAX_FILE_BYTES, Origin};

/// 文件来源的拥有所有权的读取选项；不缓存原始文件内容。
#[derive(Clone, Debug)]
pub(super) struct FileInput {
    /// 加载时结合上下文解析的文件路径，属于可展示的定位元数据。
    pub(super) path: PathBuf,
    /// 是否只将文件不存在视为该层缺席；其他错误仍须返回。
    pub(super) optional: bool,
}

impl FileInput {
    /// 保存文件路径，默认要求文件存在，不在此时执行 I/O。
    pub(super) fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            optional: false,
        }
    }

    /// 按本次固定基础目录读取完整 UTF-8 文本，并返回已解析路径的来源。
    ///
    /// `None` 只表示可选文件不存在；调用方应据此返回不带伪造来源的空层。
    /// 读取、编码或资源限制失败都不交付部分文件内容。
    pub(super) fn read(
        &self,
        context: &LoadContext,
    ) -> Result<(Option<String>, Origin), ConfigError> {
        let path = context.resolve_path(&self.path);
        let origin = Origin::file(path.clone());
        // 命名管道可能在 open 本身阻塞，因此必须先拒绝静态非普通文件路径。
        // 打开后仍检查实际句柄；这里不承诺抵御并发恶意替换路径的竞态。
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if self.optional && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((None, origin));
            }
            Err(_) => return Err(ConfigError::new(ConfigErrorKind::Io).with_origin(origin)),
        };
        if !metadata.is_file() {
            return Err(ConfigError::new(ConfigErrorKind::Io).with_origin(origin));
        }
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(ConfigError::new(ConfigErrorKind::Limit).with_origin(origin));
        }
        let file = match File::open(&path) {
            Ok(file) => file,
            Err(error) if self.optional && error.kind() == std::io::ErrorKind::NotFound => {
                return Ok((None, origin));
            }
            Err(_) => return Err(ConfigError::new(ConfigErrorKind::Io).with_origin(origin)),
        };
        // 路径检查后文件可能发生变化，继续核对实际打开对象的类型与当前大小。
        let metadata = file
            .metadata()
            .map_err(|_| ConfigError::new(ConfigErrorKind::Io).with_origin(origin.clone()))?;
        if !metadata.is_file() {
            return Err(ConfigError::new(ConfigErrorKind::Io).with_origin(origin));
        }
        if metadata.len() > MAX_FILE_BYTES as u64 {
            return Err(ConfigError::new(ConfigErrorKind::Limit).with_origin(origin));
        }
        let mut bytes = Vec::new();
        // 元数据中的长度不是读取期间的永久保证。最多多读一个字节来判断是否越界，
        // 避免文件增长时 read_to_end 不受限制地扩展缓冲区。
        file.take(MAX_FILE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ConfigError::new(ConfigErrorKind::Io).with_origin(origin.clone()))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(ConfigError::new(ConfigErrorKind::Limit).with_origin(origin));
        }
        let text = String::from_utf8(bytes)
            .map_err(|_| ConfigError::new(ConfigErrorKind::Encoding).with_origin(origin.clone()))?;
        Ok((Some(text), origin))
    }
}

/// 一次扫描得到的行首字节偏移，用于把解析器 span 映射为来源行列。
///
/// 列按字节计数，与解析器偏移保持一致；不将 UTF-8 字节偏移冒充显示字符宽度。
#[cfg(any(feature = "json", feature = "toml"))]
pub(super) struct Positions {
    line_starts: Vec<usize>,
    origin: Origin,
}

#[cfg(any(feature = "json", feature = "toml"))]
impl Positions {
    /// 记录每行起点，避免为每个配置节点重新扫描整个文件前缀。
    pub(super) fn new(text: &str, origin: Origin) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(
            text.bytes()
                .enumerate()
                .filter_map(|(index, byte)| (byte == b'\n').then_some(index + 1)),
        );
        Self {
            line_starts,
            origin,
        }
    }

    /// 将来自同一文本的合法字节偏移转换为从 1 开始的行、列。
    ///
    /// 二分定位所属行；若来源位置构造失败，则只保留文件来源，不伪造精确位置。
    pub(super) fn at(&self, offset: usize) -> Origin {
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        self.origin
            .clone()
            .at(line + 1, offset - self.line_starts[line] + 1)
            .unwrap_or_else(|_| self.origin.clone())
    }
}
