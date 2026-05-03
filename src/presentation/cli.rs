pub mod agent_cli;
pub mod chat_cli;
pub mod digest_cli;
pub mod research_cli;
pub mod serve_cli;
pub mod survey_cli;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Parser, Debug)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    Serve {
        #[arg(long, default_value = "0.0.0.0:3000")]
        addr: SocketAddr,
    },
    Chat {
        #[arg(long, default_value = "http://localhost:3000")]
        base_url: String,

        #[arg(long)]
        session_id: Option<Uuid>,
    },
    Agent,
    Research,
    /// Read and summarize an academic paper from a PDF file or URL
    Survey {
        /// Path to a PDF file or URL (e.g. https://arxiv.org/pdf/...)
        source: String,
        /// Output path for the markdown report (default: outputs/survey/{timestamp}.md)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Curate daily papers and tech news into a digest
    Digest {
        /// Date to fetch (YYYY-MM-DD, default: today)
        #[arg(long, short)]
        date: Option<String>,
        /// Output path for the markdown digest (default: outputs/digest/{date}.md)
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
}
