use serde::Deserialize;
use zed_extension_api::{
    self as zed,
    http_client::{self, HttpMethod, HttpRequestBuilder},
    Extension, SlashCommand, SlashCommandOutput, SlashCommandOutputSection, Worktree,
};

/// Maximum file size allowed (50 KB).
const MAX_FILE_SIZE: u64 = 50 * 1024;

/// Maximum number of files allowed.
const MAX_FILES: usize = 500;

/// Default ignore patterns for various file types and directories.
const DEFAULT_IGNORE_PATTERNS: [&str; 34] = [
    // Python
    "*.pyc",
    "*.pyo",
    "*.pyd",
    "__pycache__",
    ".pytest_cache",
    ".coverage",
    ".tox",
    ".nox",
    ".mypy_cache",
    ".ruff_cache",
    ".hypothesis",
    "poetry.lock",
    "Pipfile.lock",
    // JavaScript/Node
    "node_modules",
    "bower_components",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    ".npm",
    ".yarn",
    ".pnpm-store",
    // Rust
    "Cargo.lock",
    "target",
    // Version control
    ".git",
    ".svn",
    ".hg",
    ".gitignore",
    ".gitattributes",
    ".gitmodules",
    // Virtual environments
    "venv",
    ".venv",
    "env",
    ".env",
    "virtualenv",
];

/// Represents the content of a GitHub repository.
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
/// Represents the content of a GitHub repository.
struct GitHubContent {
    /// The name of the content (file or directory).
    name: String,
    /// The path of the content within the repository.
    path: String,
    /// The type of the content (e.g., file, directory).
    #[serde(rename = "type")]
    content_type: String,
    /// The size of the content in bytes, if applicable.
    size: Option<u64>,
    /// The SHA hash of the content.
    sha: String,
    /// The API URL to access the content.
    url: String,
    /// The HTML URL to view the content on GitHub.
    html_url: String,
    /// The Git URL to access the content.
    git_url: String,
    /// The download URL for the content, if available.
    download_url: Option<String>,
    /// Additional links related to the content.
    #[serde(rename = "_links")]
    links: serde_json::Value,
    /// The actual content of the file, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    /// The encoding of the content, if applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<String>,
}

/// Type alias for a vector of GitHubContent.
type GitHubContents = Vec<GitHubContent>;

/// Represents errors that can occur when interacting with the GitHub API.
#[derive(Debug)]
/// Represents errors that can occur when interacting with the GitHub API.
enum GitHubApiError {
    /// Error indicating a network-related issue.
    Network(String),
    /// Error indicating an unspecified issue.
    Other(String),
}

impl std::fmt::Display for GitHubApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(msg) => write!(f, "Network error: {}", msg),
            Self::Other(msg) => write!(f, "Error: {}", msg),
        }
    }
}

/// Represents errors that can occur during the Git ingestion process.
#[derive(Debug)]
enum GitIngestError {
    /// Error indicating that the maximum allowed size has been exceeded.
    MaxSizeExceeded(u64),
    /// Error indicating that the repository was not found.
    RepoNotFound(String),
    /// Error indicating that the provided URL is invalid.
    InvalidUrl(String),
}

impl std::error::Error for GitIngestError {}

impl std::fmt::Display for GitIngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::MaxSizeExceeded(size) => write!(f, "Max size exceeded: {} bytes", size),
            Self::RepoNotFound(msg) => write!(f, "Repository not found: {}", msg),
            Self::InvalidUrl(url) => write!(f, "Invalid URL: {}", url),
        }
    }
}

/// Represents a node in the repository tree structure.
#[derive(Debug)]
struct Node {
    /// The name of the node (file or directory).
    name: String,
    /// The type of the node (e.g., file, directory).
    r#type: String,
    /// The size of the node in bytes.
    size: u64,
    /// The children nodes of this node.
    children: Vec<Node>,
    /// The number of files in this node.
    file_count: usize,
    /// The number of directories in this node.
    dir_count: usize,
    /// The path of the node within the repository.
    path: String,
    /// Indicates whether the content of the node should be ignored.
    ignore_content: bool,
    /// The download URL for the node, if available.
    download_url: Option<String>,
}

/// Represents a query to fetch repository contents.
#[derive(Debug)]
struct Query {
    /// The username of the repository owner.
    user_name: String,
    /// The name of the repository.
    repo_name: String,
    /// The path within the repository to fetch contents from.
    path: Option<String>,
    /// The branch of the repository to fetch contents from.
    branch: Option<String>,
    /// The patterns to ignore from repository analysis.
    ignore_patterns: Vec<String>,
    /// The patterns to exclude from repository analysis.
    exclude_patterns: Vec<String>,
    /// The patterns to include in repository analysis.
    include_patterns: Vec<String>,
}

impl Query {
    /// Creates a new Query from a GitHub URL.
    ///
    /// # Arguments
    /// * `url` - A GitHub repository URL in the format "https://github.com/username/repo"
    ///          optionally including branch and path information
    ///
    /// # Returns
    /// * `Ok(Query)` - Successfully parsed Query object containing repository details
    /// * `Err(GitIngestError)` - Error if URL is invalid or not a GitHub repository URL
    ///
    /// # Example URL Formats
    /// - Basic: https://github.com/username/repo
    /// - With branch: https://github.com/username/repo/tree/branch
    /// - With path: https://github.com/username/repo/tree/branch/path/to/dir
    fn new(url: &str) -> Result<Self, GitIngestError> {
        let url = url.trim();
        if !url.to_lowercase().starts_with("https://github.com/") {
            return Err(GitIngestError::InvalidUrl("Not a GitHub URL".to_string()));
        }

        let url = url
            .strip_prefix("https://github.com/")
            .ok_or_else(|| GitIngestError::InvalidUrl("Not a GitHub URL".to_string()))?;
        let segments: Vec<&str> = url.split('/').filter(|s| !s.is_empty()).collect();

        if segments.len() < 2 {
            return Err(GitIngestError::InvalidUrl(
                "Invalid repository URL".to_string(),
            ));
        }

        let user_name = segments[0].to_string();
        let repo_name = segments[1].to_string();

        let branch = if segments.len() > 3 {
            match segments[2] {
                "tree" => Some(segments[3].to_string()),
                "blob" => Some(segments[3].to_string()),

                _ => None,
            }
        } else {
            None
        };

        let path = if segments.len() > 4 {
            Some(segments[4..].join("/"))
        } else {
            None
        };

        Ok(Self {
            user_name,
            repo_name,
            path,
            branch,
            ignore_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            include_patterns: Vec::new(),
        })
    }

    /// Constructs the API URL for fetching repository contents.
    ///
    /// This method builds the GitHub API URL by combining:
    /// - Base GitHub API URL
    /// - Repository owner and name
    /// - Optional path within repository
    /// - Optional branch reference
    ///
    /// # Returns
    /// A String containing the complete GitHub API URL for fetching contents
    fn api_url(&self, path: &str) -> String {
        let mut url = format!(
            "https://api.github.com/repos/{}/{}/contents",
            self.user_name, self.repo_name
        );

        if !path.is_empty() {
            url.push_str(&format!("/{}", path));
        }

        if let Some(ref branch) = self.branch {
            url.push_str(&format!("?ref={}", branch));
        }

        url
    }

    /// Processes the repository and returns a summary string.
    ///
    /// # Returns
    /// * `Ok(String)` - Summary string of repository analysis
    /// * `Err(GitIngestError)` - Error if processing fails
    async fn process_repo(&mut self) -> Result<String, GitIngestError> {
        let mut stats = Stats {
            total_files: 0,
            total_size: 0,
        };

        // Fetch initial repository contents
        let contents = self
            .fetch_contents(self.path.as_deref().unwrap_or(""))
            .await
            .map_err(|e| GitIngestError::RepoNotFound(e.to_string()))?;

        // Create root node
        let mut root_node = Node {
            name: self.repo_name.clone(),
            r#type: "directory".to_string(),
            size: 0,
            children: vec![],
            file_count: 0,
            dir_count: 1,
            path: String::new(),
            ignore_content: false,
            download_url: None,
        };

        if !self.ignore_patterns.is_empty() {
            self.set_ignore_patterns().await;
        }

        // Process all contents
        for content in contents {
            if !self.should_include(&content.path) || self.should_exclude(&content.path) {
                continue;
            }

            match content.content_type.as_str() {
                "file" => {
                    if let Some(value) = self.process_file(&mut stats, &mut root_node, &content) {
                        return value;
                    }
                }
                "dir" => {
                    if let Some(value) = self
                        .process_directory(&mut stats, &mut root_node, content)
                        .await
                    {
                        return value;
                    }
                }
                _ => continue,
            }
        }

        root_node.file_count = stats.total_files;
        root_node.dir_count = root_node
            .children
            .iter()
            .filter(|n| n.r#type == "directory")
            .count();
        root_node.size = stats.total_size;

        let mut files = Vec::new();
        match self.extract_files_content(&root_node, &mut files).await {
            Ok(_) => {}
            Err(e) => return Err(GitIngestError::RepoNotFound(e.to_string())),
        };

        Ok(self.create_summary_string(&root_node, &files))
    }

    /// Creates a summary string for the repository.
    ///
    /// # Arguments
    /// * `nodes` - Root node containing repository tree structure
    /// * `files` - List of file contents to include in summary
    ///
    /// # Returns
    /// A formatted string containing repository summary with directory structure and file contents
    fn create_summary_string(&self, nodes: &Node, files: &[FileContent]) -> String {
        let mut summary = String::new();
        summary.push_str(&format!(
            "Repository: {}/{}\n",
            self.user_name, self.repo_name
        ));
        summary.push_str(&format!("Total files: {}\n", nodes.file_count));
        summary.push_str(&format!(
            "Total size: {:.2} KB\n",
            nodes.size as f64 / 1024.0
        ));
        summary.push_str(&format!("Directory count: {}\n\n", nodes.dir_count));

        let path = self.path.as_deref().unwrap_or("");
        let api_url = self.api_url(path);
        summary.push_str("API URL:\n");
        summary.push_str(&format!("{}\n\n", api_url));

        summary.push_str("Directory structure:\n");
        summary.push_str(&self.create_tree_structure(nodes, "", true));

        if !files.is_empty() {
            summary.push_str("\nSelected file contents:\n");
            for file in files {
                summary.push_str(&format!("\n--- {} ---\n{}\n", file.path, file.content));
            }
        }

        summary
    }

    /// Creates a tree structure string representation of the repository.
    ///
    /// # Arguments
    /// * `node` - Current node in the repository tree structure
    /// * `prefix` - String prefix to use for the current node's indentation
    /// * `is_last` - Whether this node is the last child in its parent's children
    ///
    /// # Returns
    /// A formatted string representing the node and its children in a tree structure
    fn create_tree_structure(&self, node: &Node, prefix: &str, is_last: bool) -> String {
        let mut result = String::new();
        let marker = if is_last { "└── " } else { "├── " };

        result.push_str(&format!("{}{}{}\n", prefix, marker, node.name));

        let child_prefix = if is_last { "    " } else { "│   " };
        for (i, child) in node.children.iter().enumerate() {
            result.push_str(&self.create_tree_structure(
                child,
                &format!("{}{}", prefix, child_prefix),
                i == node.children.len() - 1,
            ));
        }

        result
    }

    /// Extracts the content of files from the repository.
    ///
    /// # Arguments
    /// * `node` - Current node in the repository tree structure
    /// * `files` - Vector to store extracted file contents
    ///
    /// # Returns
    /// * `Ok(())` - Successfully extracted file contents
    /// * `Err(GitHubApiError)` - Error if content extraction fails
    async fn extract_files_content(
        &self,
        node: &Node,
        files: &mut Vec<FileContent>,
    ) -> Result<(), GitHubApiError> {
        if node.r#type == "file" && !node.ignore_content {
            if let Some(download_url) = &node.download_url {
                if let Ok(content) = fetch_file_content(download_url).await {
                    files.push(FileContent {
                        path: node.path.clone(),
                        content,
                    });
                }
            }
        }

        for child in &node.children {
            Box::pin(self.extract_files_content(child, files)).await?;
        }

        Ok(())
    }

    /// Sets ignore patterns for the repository processing.
    ///
    /// This method initializes the ignore patterns with default patterns and
    /// optionally incorporates patterns from repository's .gitignore file if present.
    ///
    /// # Returns
    /// None, but updates the ignore_patterns vector with default patterns and
    /// any additional patterns found in .gitignore.
    async fn set_ignore_patterns(&mut self) {
        self.ignore_patterns = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|&s| s.to_string())
            .collect();

        let gitignore_result = self.fetch_contents(".gitignore").await;
        if let Ok(ignore_content) = gitignore_result {
            if let Some(download_url) = ignore_content.first().and_then(|c| c.download_url.as_ref())
            {
                if let Ok(gitignore) = fetch_file_content(download_url).await {
                    let patterns = gitignore
                        .lines()
                        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
                        .map(ToString::to_string);
                    self.ignore_patterns.extend(patterns);
                }
            }
        }
    }

    /// Processes a file node in the repository tree.
    ///
    /// # Arguments
    /// * `stats` - Statistics tracking struct for the repository
    /// * `root_node` - Parent node to attach file node to
    /// * `content` - GitHub content information for the file
    ///
    /// # Returns
    /// * `Some(Result<String, GitIngestError>)` if an error occurred
    /// * `None` if processing completed successfully
    fn process_file(
        &self,
        stats: &mut Stats,
        root_node: &mut Node,
        content: &GitHubContent,
    ) -> Option<Result<String, GitIngestError>> {
        stats.total_files += 1;
        let size = content.size.unwrap_or(0);
        stats.total_size += size;
        if stats.total_files > MAX_FILES {
            return Some(Err(GitIngestError::MaxSizeExceeded(stats.total_size)));
        }
        root_node.children.push(Node {
            name: content.name.clone(),
            r#type: "file".to_string(),
            size,
            children: vec![],
            file_count: 1,
            dir_count: 0,
            path: content.path.clone(),
            ignore_content: size > MAX_FILE_SIZE,
            download_url: content.download_url.clone(),
        });

        None
    }

    /// Processes a directory node in the repository tree.
    ///
    /// # Arguments
    /// * `stats` - Statistics tracking struct for the repository
    /// * `root_node` - Parent node to attach directory node to
    /// * `content` - GitHub content information for the directory
    ///
    /// # Returns
    /// * `Some(Result<String, GitIngestError>)` if an error occurred
    /// * `None` if processing completed successfully
    async fn process_directory(
        &self,
        stats: &mut Stats,
        root_node: &mut Node,
        content: GitHubContent,
    ) -> Option<Result<String, GitIngestError>> {
        if let Ok(dir_contents) = self.fetch_contents(&content.path).await {
            let mut dir_node = Node {
                name: content.name,
                r#type: "directory".to_string(),
                size: 0,
                children: vec![],
                file_count: 0,
                dir_count: 1,
                path: content.path,
                ignore_content: false,
                download_url: content.download_url.clone(),
            };

            // Process directory contents
            for item in dir_contents {
                if !self.should_include(&item.path) || self.should_exclude(&item.path) {
                    continue;
                }

                if item.content_type == "file" {
                    if let Some(value) = self.process_file(stats, &mut dir_node, &item) {
                        return Some(value);
                    }
                } else if item.content_type == "dir" {
                    if let Some(value) =
                        Box::pin(self.process_directory(stats, &mut dir_node, item)).await
                    {
                        return Some(value);
                    }
                }
            }

            root_node.children.push(dir_node);
        }
        None
    }

    /// Fetches the contents of the repository.
    ///
    /// # Arguments
    /// * `path` - Optional path within the repository to fetch contents from
    /// * `rate_limit` - Optional rate limit tracking for GitHub API requests
    ///
    /// # Returns
    /// * `Ok(GitHubContents)` - Repository contents at specified path
    /// * `Err(GitHubApiError)` - Error if fetch fails or rate limit exceeded
    async fn fetch_contents(&self, path: &str) -> Result<GitHubContents, GitHubApiError> {
        let url = if !path.is_empty() {
            self.api_url(path)
        } else {
            self.api_url("")
        };

        let request = HttpRequestBuilder::new()
            .header("User-Agent", "X-GitHub-Api-Version: 2022-11-28")
            .header("Accept", "application/vnd.github+json")
            .method(HttpMethod::Get)
            .url(&url)
            .build()
            .map_err(|e| GitHubApiError::Network(e.to_string()));

        let resp =
            http_client::fetch(&request?).map_err(|e| GitHubApiError::Network(e.to_string()))?;

        let content: GitHubContents =
            serde_json::from_slice(&resp.body).map_err(|e| GitHubApiError::Other(e.to_string()))?;

        Ok(content)
    }

    /// Helper function to check if file path matches include patterns.
    ///
    /// # Arguments
    /// * `path` - Path within repository to check for inclusion
    ///
    /// # Returns
    /// * `true` if path matches include patterns or if no include patterns are set
    /// * `false` if path does not match any include patterns
    fn should_include(&self, path: &str) -> bool {
        if self.include_patterns.is_empty() {
            return true;
        }
        self.include_patterns
            .iter()
            .any(|pattern| glob::Pattern::new(pattern).map_or(false, |p| p.matches(path)))
    }

    /// Helper function to check if file path matches exclude or ignore patterns.
    ///
    /// # Arguments
    /// * `path` - Path within repository to check for exclusion
    ///
    /// # Returns
    /// * `true` if path matches exclude patterns or ignore patterns
    /// * `false` if path does not match any exclude or ignore patterns
    fn should_exclude(&self, path: &str) -> bool {
        self.exclude_patterns
            .iter()
            .any(|pattern| glob::Pattern::new(pattern).map_or(false, |p| p.matches(path)))
            || self
                .ignore_patterns
                .iter()
                .any(|p| path.contains(p.as_str()))
    }
}

/// Represents the content of a file in the repository.
/// Represents the content of a file in the repository.
#[derive(Debug)]
struct FileContent {
    /// The path of the file within the repository.
    path: String,
    /// The actual content of the file.
    content: String,
}

/// Represents statistics about the repository.
#[derive(Debug)]
struct Stats {
    /// The total number of files in the repository.
    total_files: usize,
    /// The total size of all files in the repository, in bytes.
    total_size: u64,
}

/// Fetches the content of a file from a given URL.
///
/// # Arguments
/// * `url` - The URL to fetch the file content from
/// * `rate_limit` - Optional rate limit tracking for GitHub API requests
///
/// # Returns
/// * `Ok(String)` - The content of the file as a UTF-8 string
/// * `Err(GitHubApiError)` - Error if the fetch fails or rate limit is exceeded
async fn fetch_file_content(url: &str) -> Result<String, GitHubApiError> {
    let request = HttpRequestBuilder::new()
        .header("User-Agent", "X-GitHub-Api-Version: 2022-11-28")
        .header("Accept", "application/vnd.github.raw")
        .method(HttpMethod::Get)
        .url(url)
        .build()
        .map_err(|e| GitHubApiError::Network(e.to_string()));

    let resp = http_client::fetch(&request?).map_err(|e| GitHubApiError::Network(e.to_string()))?;

    String::from_utf8(resp.body).map_err(|e| GitHubApiError::Other(e.to_string()))
}

/// Represents the GitIngest extension that analyzes GitHub repositories.
#[derive(Default)]
struct GitIngestExtension;

impl Extension for GitIngestExtension {
    /// Creates a new instance of GitIngestExtension.
    fn new() -> Self {
        Self
    }

    /// Executes a slash command to analyze GitHub repositories.
    ///
    /// # Arguments
    /// * `command` - The slash command to execute
    /// * `args` - Vector of command arguments, expecting a GitHub repository URL
    /// * `_worktree` - Optional worktree reference (unused)
    ///
    /// # Returns
    /// * `Ok(SlashCommandOutput)` containing repository analysis results
    /// * `Err(String)` if command fails or repository URL is invalid
    fn run_slash_command(
        &self,
        command: SlashCommand,
        args: Vec<String>,
        _worktree: Option<&Worktree>,
    ) -> Result<SlashCommandOutput, String> {
        match command.name.as_str() {
            "gitingest" => {
                if args.is_empty() {
                    return Err("nothing to ingest".to_string());
                }

                let url = args.first().unwrap();
                let mut query =
                    Query::new(url).map_err(|e| format!("invalid repository url: {e}"))?;

                let mut exclude_patterns = Vec::new();
                let mut include_patterns = Vec::new();

                for arg in args.iter().skip(1) {
                    if arg.starts_with("exclude:") {
                        arg.trim_start_matches("exclude:")
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .for_each(|s| exclude_patterns.push(s));
                    } else if arg.starts_with("include:") {
                        arg.trim_start_matches("include:")
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .for_each(|s| include_patterns.push(s));
                    }
                }

                query.exclude_patterns = exclude_patterns;
                query.include_patterns = include_patterns;

                let text = match futures::executor::block_on(query.process_repo()) {
                    Ok(summary) => summary,
                    Err(e) => format!("error: {e}"),
                };

                Ok(SlashCommandOutput {
                    sections: vec![SlashCommandOutputSection {
                        range: (0..text.len()).into(),
                        label: "GitHub Repository Code".to_string(),
                    }],
                    text,
                })
            }
            command => Err(format!("unknown slash command: \"{command}\"")),
        }
    }
}

zed::register_extension!(GitIngestExtension);

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(test)]
    fn create_mock_github_content(
        name: &str,
        path: &str,
        content_type: &str,
        size: Option<u64>,
        download_url: Option<String>,
    ) -> GitHubContent {
        GitHubContent {
            name: name.to_string(),
            path: path.to_string(),
            content_type: content_type.to_string(),
            size,
            sha: "mock_sha".to_string(),
            url: "https://api.github.com/mock".to_string(),
            html_url: "https://github.com/mock".to_string(),
            git_url: "git://github.com/mock.git".to_string(),
            download_url,
            links: serde_json::json!({}),
            content: None,
            encoding: None,
        }
    }

    #[test]
    fn test_query_new_valid_url() {
        let url = "https://github.com/username/repo";
        let query = Query::new(url).unwrap();
        assert_eq!(query.user_name, "username");
        assert_eq!(query.repo_name, "repo");
        assert!(query.path.is_none());
        assert!(query.branch.is_none());
    }

    #[test]
    fn test_query_new_with_branch() {
        let url = "https://github.com/username/repo/tree/main";
        let query = Query::new(url).unwrap();
        assert_eq!(query.user_name, "username");
        assert_eq!(query.repo_name, "repo");
        assert!(query.path.is_none());
        assert_eq!(query.branch.unwrap(), "main");
    }

    #[test]
    fn test_query_new_with_path() {
        let url = "https://github.com/username/repo/tree/main/src";
        let query = Query::new(url).unwrap();
        assert_eq!(query.user_name, "username");
        assert_eq!(query.repo_name, "repo");
        assert_eq!(query.path.unwrap(), "src");
        assert_eq!(query.branch.unwrap(), "main");
    }

    #[test]
    fn test_query_new_invalid_url() {
        let url = "not_a_github_url";
        let result = Query::new(url);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GitIngestError::InvalidUrl(_)));
    }

    #[test]
    fn test_create_summary_string() {
        let query = Query {
            user_name: String::from("test_user"),
            repo_name: String::from("test_repo"),
            path: None,
            branch: None,
            ignore_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            include_patterns: Vec::new(),
        };

        let root_node = Node {
            name: String::from("test_repo"),
            r#type: String::from("directory"),
            size: 1024,
            children: vec![],
            file_count: 5,
            dir_count: 2,
            path: String::new(),
            ignore_content: false,
            download_url: None,
        };

        let files = vec![FileContent {
            path: String::from("test.txt"),
            content: String::from("test content"),
        }];

        let summary = query.create_summary_string(&root_node, &files);
        assert!(summary.contains("test_user/test_repo"));
        assert!(summary.contains("Total files: 5"));
        assert!(summary.contains("Total size: 1.00 KB"));
        assert!(summary.contains("Directory count: 2"));
    }

    #[test]
    fn test_create_tree_structure() {
        let query = Query {
            user_name: String::from("test_user"),
            repo_name: String::from("test_repo"),
            path: None,
            branch: None,
            ignore_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            include_patterns: Vec::new(),
        };

        let child_node = Node {
            name: String::from("child"),
            r#type: String::from("file"),
            size: 100,
            children: vec![],
            file_count: 1,
            dir_count: 0,
            path: String::from("child"),
            ignore_content: false,
            download_url: None,
        };

        let root_node = Node {
            name: String::from("root"),
            r#type: String::from("directory"),
            size: 100,
            children: vec![child_node],
            file_count: 1,
            dir_count: 1,
            path: String::new(),
            ignore_content: false,
            download_url: None,
        };

        let tree = query.create_tree_structure(&root_node, "", true);
        assert!(tree.contains("└── root"));
        assert!(tree.contains("    └── child"));
    }

    #[test]
    fn test_extension_run_invalid_command() {
        let extension = GitIngestExtension::new();
        let result = extension.run_slash_command(
            SlashCommand {
                name: String::from("invalid"),
                description: String::from("Test invalid command"),
                tooltip_text: String::from("Test invalid command tooltip"),
                requires_argument: false,
            },
            vec![],
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_github_api_error_display() {
        let network = GitHubApiError::Network("connection failed".to_string());
        let other = GitHubApiError::Other("unknown error".to_string());

        assert_eq!(network.to_string(), "Network error: connection failed");
        assert_eq!(other.to_string(), "Error: unknown error");
    }

    #[test]
    fn test_git_ingest_error_display() {
        let max_size = GitIngestError::MaxSizeExceeded(1000);
        let not_found = GitIngestError::RepoNotFound("repo missing".to_string());
        let invalid_url = GitIngestError::InvalidUrl("bad url".to_string());

        assert_eq!(max_size.to_string(), "Max size exceeded: 1000 bytes");
        assert_eq!(not_found.to_string(), "Repository not found: repo missing");
        assert_eq!(invalid_url.to_string(), "Invalid URL: bad url");
    }

    #[test]
    fn test_node_creation() {
        let mock_content = create_mock_github_content(
            "test.txt",
            "test/test.txt",
            "file",
            Some(100),
            Some("https://test.com/download".to_string()),
        );

        let node = Node {
            name: mock_content.name,
            r#type: "file".to_string(),
            size: mock_content.size.unwrap(),
            children: vec![],
            file_count: 1,
            dir_count: 0,
            path: mock_content.path,
            ignore_content: mock_content.size.unwrap() > MAX_FILE_SIZE,
            download_url: mock_content.download_url,
        };

        assert_eq!(node.name, "test.txt");
        assert_eq!(node.r#type, "file");
        assert_eq!(node.size, 100);
        assert_eq!(node.file_count, 1);
        assert_eq!(node.dir_count, 0);
        assert_eq!(node.path, "test/test.txt");
        assert!(!node.ignore_content);
        assert_eq!(node.download_url.unwrap(), "https://test.com/download");
    }

    #[test]
    fn test_query_api_url_construction() {
        let query = Query {
            user_name: "test-user".to_string(),
            repo_name: "test-repo".to_string(),
            path: Some("src".to_string()),
            branch: Some("main".to_string()),
            ignore_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            include_patterns: Vec::new(),
        };

        let path = query.path.as_deref().unwrap_or("");
        let api_url = query.api_url(path);
        assert!(api_url.starts_with("https://api.github.com/repos/"));
        assert!(api_url.contains("test-user/test-repo"));
        assert!(api_url.contains("/src"));
        assert!(api_url.contains("?ref=main"));
    }

    #[test]
    fn test_ignore_patterns_filtering() {
        let patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .map(|&s| s.to_string())
            .collect();

        let mock_files = vec![
            "node_modules/package.json",
            ".git/config",
            "__pycache__/cache.pyc",
            "src/main.rs",
            "docs/README.md",
        ];

        for file in mock_files {
            let should_ignore = patterns.iter().any(|p| file.contains(p));

            if file.contains("node_modules")
                || file.contains(".git")
                || file.contains("__pycache__")
            {
                assert!(should_ignore, "Should ignore {}", file);
            } else {
                assert!(!should_ignore, "Should not ignore {}", file);
            }
        }
    }

    #[test]
    fn test_include_exclude_patterns() {
        let query = Query {
            user_name: String::from("test_user"),
            repo_name: String::from("test_repo"),
            path: None,
            branch: None,
            ignore_patterns: Vec::new(),
            exclude_patterns: vec!["test/*.log".to_string()],
            include_patterns: vec!["**/*.rs".to_string()],
        };

        // Mock files to test against patterns
        let mock_files = vec![
            "src/main.rs",
            "src/lib.rs",
            "test/output.log",
            "docs/index.html",
            "test/test.rs",
        ];

        // Helper functions matching those in process_repo
        let should_include = |path: &str| -> bool {
            if query.include_patterns.is_empty() {
                return true;
            }
            query
                .include_patterns
                .iter()
                .any(|pattern| glob::Pattern::new(pattern).map_or(false, |p| p.matches(path)))
        };

        let should_exclude = |path: &str| -> bool {
            query
                .exclude_patterns
                .iter()
                .any(|pattern| glob::Pattern::new(pattern).map_or(false, |p| p.matches(path)))
        };

        // Test matching
        for path in mock_files {
            let included = should_include(path);
            let excluded = should_exclude(path);

            match path {
                // Should be included (*.rs) and not excluded
                "src/main.rs" | "src/lib.rs" => {
                    assert!(included, "Should include {}", path);
                    assert!(!excluded, "Should not exclude {}", path);
                }
                // Should be included (*.rs) but also excluded (test/*.log)
                "test/output.log" => {
                    assert!(!included, "Should not include {}", path);
                    assert!(excluded, "Should exclude {}", path);
                }
                // Should be neither included nor excluded
                "docs/index.html" => {
                    assert!(!included, "Should not include {}", path);
                    assert!(!excluded, "Should not exclude {}", path);
                }
                // Should be included (*.rs) and not excluded
                "test/test.rs" => {
                    assert!(included, "Should include {}", path);
                    assert!(!excluded, "Should not exclude {}", path);
                }
                _ => panic!("Unexpected test file"),
            }
        }
    }

    #[test]
    fn test_empty_patterns() {
        let query = Query {
            user_name: String::from("test_user"),
            repo_name: String::from("test_repo"),
            path: None,
            branch: None,
            ignore_patterns: Vec::new(),
            exclude_patterns: Vec::new(),
            include_patterns: Vec::new(),
        };

        let should_include = |path: &str| -> bool {
            if query.include_patterns.is_empty() {
                return true;
            }
            query
                .include_patterns
                .iter()
                .any(|pattern| glob::Pattern::new(pattern).map_or(false, |p| p.matches(path)))
        };

        let should_exclude = |path: &str| -> bool {
            query
                .exclude_patterns
                .iter()
                .any(|pattern| glob::Pattern::new(pattern).map_or(false, |p| p.matches(path)))
        };

        let test_paths = vec!["src/main.rs", "test/file.txt", "docs/index.html"];

        for path in test_paths {
            assert!(
                should_include(path),
                "Should include all files when include_patterns is empty"
            );
            assert!(
                !should_exclude(path),
                "Should exclude no files when exclude_patterns is empty"
            );
        }
    }
}
