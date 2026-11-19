use serde::Serialize;

pub type Result<T> = std::result::Result<T, StudioError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioError {
    pub status: u16,
    pub message: String,
}

impl StudioError {
    pub fn new(status: u16, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

impl From<rusqlite::Error> for StudioError {
    fn from(error: rusqlite::Error) -> Self {
        if matches!(error, rusqlite::Error::QueryReturnedNoRows) {
            return Self::new(404, "没有找到对应记录");
        }
        Self::new(500, format!("项目数据库写入或读取失败：{error}"))
    }
}

impl From<std::io::Error> for StudioError {
    fn from(error: std::io::Error) -> Self {
        Self::new(500, format!("无法读写本机文件：{error}"))
    }
}

impl From<serde_json::Error> for StudioError {
    fn from(_: serde_json::Error) -> Self {
        Self::new(400, "项目或配置的数据格式有误")
    }
}
