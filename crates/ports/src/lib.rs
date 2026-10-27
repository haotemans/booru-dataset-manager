#![forbid(unsafe_code)]
use bdm_domain::media::MediaId;
use thiserror::Error;
#[derive(Debug, Error)]
pub enum PortError {
    #[error("adapter is not implemented")]
    NotImplemented,
    #[error("adapter failed: {0}")]
    Adapter(String),
}
pub trait ExactHashLookup {
    fn find_by_md5(&self, md5_hex: &str) -> Result<Option<MediaId>, PortError>;
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum IngestError {
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error("下载已取消，已入库的图片会保留。")]
    Cancelled,
    #[error("原图地址不受支持，已停止请求。")]
    InvalidUrl,
    #[error("原图格式、尺寸或文件大小不符合下载规则。")]
    InvalidMedia,
    #[error("原图校验失败，文件未入库。")]
    Integrity,
    #[error("Danbooru 限流，等待 {0} 秒后重试。")]
    RateLimited(u64),
    #[error("图片存储操作失败，请检查目录权限及剩余空间。")]
    Storage,
    #[error("数据库暂时被其他任务占用，已保留进度，请继续任务重试。")]
    DatabaseBusy,
    #[error("存储操作失败（{0}），已保留进度。")]
    StorageDetails(String),
    #[error("DINOv3 本地分析失败，请检查模型校验和 Python 运行环境后重试。")]
    Analysis,
}

pub trait MediaProvider {
    fn posts(
        &self,
        tag: &str,
        before: Option<i64>,
    ) -> Result<Vec<bdm_domain::ingest::SourcePost>, IngestError>;
    fn download(
        &self,
        post: &bdm_domain::ingest::SourcePost,
        path: &std::path::Path,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<(), IngestError>;
}

pub trait IngestRepository {
    fn save_job(&mut self, job: &bdm_domain::ingest::IngestJob) -> Result<(), IngestError>;
    fn has_source(
        &mut self,
        post: &bdm_domain::ingest::SourcePost,
        collection: i64,
    ) -> Result<bool, IngestError>;
    fn import(
        &mut self,
        post: &bdm_domain::ingest::SourcePost,
        root: &std::path::Path,
        collection: i64,
        minimum_short_side: u32,
        cancel: &std::sync::atomic::AtomicBool,
        provider: &dyn MediaProvider,
    ) -> Result<bool, IngestError>;
    // Stable port signature forwards the explicit import options to provider adapters.
    #[allow(clippy::too_many_arguments)]
    fn import_with_options(
        &mut self,
        post: &bdm_domain::ingest::SourcePost,
        root: &std::path::Path,
        collection: i64,
        minimum_short_side: u32,
        flatten_transparency: bool,
        cancel: &std::sync::atomic::AtomicBool,
        provider: &dyn MediaProvider,
    ) -> Result<bool, IngestError> {
        let _ = flatten_transparency;
        self.import(post, root, collection, minimum_short_side, cancel, provider)
    }
    fn record_outcome(&mut self, job: i64, post: i64, outcome: &str) -> Result<(), IngestError>;
}

// Deliberately neither Debug nor Serialize: secrets do not belong in logs/config.
pub struct SourceCredentials {
    pub username: String,
    pub api_key: String,
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    #[error("系统凭据存储不可用，请稍后重试。")]
    Unavailable,
    #[error("无法访问系统凭据存储，请检查当前 Windows 用户权限。")]
    Access,
    #[error("已保存的凭据格式无效，请重新填写。")]
    InvalidData,
}

pub trait SourceCredentialStore {
    fn load(&self) -> Result<Option<SourceCredentials>, CredentialError>;
    fn save(&self, credentials: &SourceCredentials) -> Result<(), CredentialError>;
    fn clear(&self) -> Result<(), CredentialError>;
}

// Kept out of serializable profile metadata and diagnostic output.
pub struct ApiCredentials {
    pub endpoint: String,
    pub api_key: String,
}

pub trait ApiCredentialStore {
    fn load(&self, profile_id: &str) -> Result<Option<ApiCredentials>, CredentialError>;
    fn save(&self, profile_id: &str, credentials: &ApiCredentials) -> Result<(), CredentialError>;
    fn clear(&self, profile_id: &str) -> Result<(), CredentialError>;
}

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum CatalogError {
    #[error("请输入单个完整 Tag，不要粘贴网址或搜索表达式。")]
    InvalidTag,
    #[error("未找到这个 Tag，请检查拼写。")]
    NotFound,
    #[error("这是画师 Tag，请到画师页添加。")]
    ArtistTag,
    #[error("这是角色 Tag，请到角色页添加。")]
    CharacterTag,
    #[error("这个 Tag 不属于画师或角色。")]
    WrongCategory,
    #[error("Danbooru 认证失败（401），请检查已保存的用户名和 API Key。")]
    Authentication,
    #[error("Danbooru 要求 Cloudflare 网站验证，当前无法查询。请在浏览器中手动检查网站访问，再返回重试；浏览器验证不保证应用请求可用。")]
    WebsiteVerification,
    #[error("Danbooru 拒绝访问（403），请检查账号权限和网站访问情况；这不一定是 API Key 错误。")]
    AccessDenied,
    #[error("Danbooru 暂时限制请求，请稍后再试。")]
    RateLimited,
    #[error("Danbooru 暂时不可用，请稍后再试。")]
    Service,
    #[error("无法连接 Danbooru，请检查网络后重试。")]
    Network,
    #[error("连接 Danbooru 超时，请重试。")]
    Timeout,
    #[error("Danbooru 返回的数据无效，请稍后重试。")]
    InvalidResponse,
    #[error("无法读取 Danbooru 凭据，请在系统设置重新保存。")]
    Credentials,
    #[error("本地数据库操作失败，请检查存储目录权限和可用空间后重启。")]
    Storage,
    #[error("数据库版本不兼容，请使用匹配版本的程序。")]
    Schema,
    #[error("删除范围已变化，请关闭后重新确认。")]
    Changed,
}

pub trait TagProvider {
    fn lookup(&self, name: &str) -> Result<Option<bdm_domain::catalog::SourceTag>, CatalogError>;
}

pub trait CollectionRepository {
    fn list(&self) -> Result<Vec<bdm_domain::catalog::CollectionRecord>, CatalogError>;
    // Atomic and idempotent by provider tag identity. Existing records are returned unchanged.
    fn add(
        &mut self,
        kind: bdm_domain::catalog::CollectionKind,
        tag: &bdm_domain::catalog::SourceTag,
    ) -> Result<bdm_domain::catalog::AddCollectionResult, CatalogError>;
    // One transaction; all supplied IDs must still exist with this kind, or nothing changes.
    fn delete_empty(
        &mut self,
        kind: bdm_domain::catalog::CollectionKind,
        ids: &[i64],
    ) -> Result<(), CatalogError>;
}
