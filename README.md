# Git Ingest Extension for Zed

A [Zed](https://zed.dev) extension that allows you to get a prompt-friendly extract of GitHub repositories via slash commands.

## Features

- **Prompt-Friendly Digests:** Generate concise text summaries from GitHub repositories optimized for LLM prompts.
- **Flexible Command Usage:** Utilize the `/gitingest` slash command with various URL and pattern options.
- **Pattern Matching:** Include or exclude specific files using glob patterns for tailored outputs.
- **Comprehensive Output:** Receive repository summaries, directory trees, selected file contents, and details of ignored files.

## Install Git Ingest Extension Locally

Follow these simple steps to set up the Git Ingest Extension in Zed Editor.

### Prerequisites

- **Zed Editor:** [Download and install](https://zed.dev/download).
- **Rust:** Install via [rustup](https://rustup.rs/).

### Steps

1. **Clone the Repository**

    ```bash
    git clone https://github.com/gc-victor/git-ingest-extension.git
    cd git-ingest-extension
    ```

2. **Install as Dev Extension**

    - Open **Zed Editor**.
    - Go to **Extensions Manager** (`Ctrl+Shift+X`).
    - Click **Install Dev Extension**.
    - Select the `git-ingest-extension` folder you cloned.

3. **Verify Installation**

    - Open the Assistant Panel with `Ctrl+?`.
    - Search for and run the `/gitingest` command.

## Usage

Use the `/gitingest` command followed by a GitHub repository URL and optional include/exclude patterns:
Basic usage with just the URL:
```bash
/gitingest https://github.com/username/repo
```

Add include/exclude patterns to filter files:
```bash 
/gitingest https://github.com/username/repo include:*.rs,src/*.ts exclude:*.log,test/*
```

For private repos or higher rate limits, add GitHub token:
```bash
/gitingest https://github.com/username/repo github_token:your_token
```

The extension supports:

URL formats:
- Basic: `https://github.com/username/repo`
- With branch: `https://github.com/username/repo/tree/branch`
- With path: `https://github.com/username/repo/tree/branch/path/to/dir`

Pattern formats:
- Include: `include:*.rs,src/*.ts` - Only include files matching these glob patterns
- Exclude: `exclude:*.log,test/*` - Exclude files matching these glob patterns
- Multiple patterns separated by commas
- Glob pattern syntax (e.g. *, **, ?)

Pattern behavior:
- If no include patterns specified, all files included by default
- Files matching exclude patterns are always excluded
- Include/exclude patterns are checked against full file paths
- Patterns are cumulative with .gitignore rules

Additional options:
- GitHub Token: `github_token:your_token` - Optional personal access token for authenticated requests
- GitHub token enables higher API rate limits and access to private repositories

## Output

The command outputs:
- Repository summary (files, size, directory count)
- Directory structure tree
- Selected file contents (within size limits)
- Ignored files/directories (based on .gitignore and exclude patterns)

## Limitations

- Maximum total size: 1MB per repository
- Maximum file size: 50KB per file
- Maximum total files: 500 files per repository
- Subject to GitHub API rate limits
- Only supports public repositories currently

## Development

Built with:
- [Rust](https://www.rust-lang.org/)
- [Zed Extension API](https://docs.rs/zed_extension_api/latest/zed_extension_api/)
- [GitHub REST API](https://docs.github.com/rest)

## Development

See the [official documentation](https://github.com/zed-industries/extensions/blob/main/docs/developing_extensions.md) for full details on extension development.

To develop locally:

1. Install Rust via rustup
2. Clone the repository
3. From Zed's extensions page, click "Install Dev Extension" and select your extension directory
4. Make changes to the extension code - Zed will automatically reload when files are modified

## Testing

Run the test suite with:

```bash
cargo test
```

## Contributing

1. Fork the repository
2. Create your feature branch
3. Commit your changes
4. Push to the branch 
5. Open a Pull Request

## Aknowledgements

Thanks to:
- [The Git Ingest project](https://gitingest.com) for inspiring this extension
- The [Zed editor team](https://github.com/zed-industries/zed) for their excellent extension API
- GitHub for providing a robust API

## License

MIT License - see LICENSE file for details