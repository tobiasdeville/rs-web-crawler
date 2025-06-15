use clap::{Parser, Subcommand};
use serde_json;
use std::time::Duration;
use tracing::{info, Level};
use tracing_subscriber;
use web_crawler::{CrawlerConfig, WebCrawler};

#[derive(Parser)]
#[command(name = "web-crawler")]
#[command(about = "A web crawler written in Rust")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Crawl a website sequentially
    Crawl {
        /// The starting URL to crawl
        url: String,
        /// Maximum crawl depth
        #[arg(short, long, default_value_t = 3)]
        depth: usize,
        /// Maximum number of pages to crawl
        #[arg(short, long)]
        max_pages: Option<usize>,
        /// Follow external links
        #[arg(short, long)]
        external: bool,
        /// Output file for results (JSON format)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Crawl a website concurrently
    ConcurrentCrawl {
        /// The starting URL to crawl
        url: String,
        /// Maximum crawl depth
        #[arg(short, long, default_value_t = 3)]
        depth: usize,
        /// Maximum concurrent requests
        #[arg(short, long, default_value_t = 10)]
        concurrent: usize,
        /// Maximum number of pages to crawl
        #[arg(short, long)]
        max_pages: Option<usize>,
        /// Follow external links
        #[arg(short, long)]
        external: bool,
        /// Output file for results (JSON format)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Crawl {
            url,
            depth,
            max_pages,
            external,
            output,
        } => {
            let config = CrawlerConfig {
                max_depth: depth,
                max_pages,
                follow_external_links: external,
                delay_between_requests: Duration::from_millis(500),
                ..Default::default()
            };

            let crawler = WebCrawler::new(config)?;
            let results = crawler.crawl(&url).await?;

            info!("Crawl completed! Found {} pages", results.len());

            if let Some(output_file) = output {
                let json = serde_json::to_string_pretty(&results)?;
                tokio::fs::write(&output_file, json).await?;
                info!("Results saved to: {}", output_file);
            } else {
                for result in &results {
                    println!("URL: {}", result.url);
                    println!("Title: {}", result.title);
                    println!("Status: {}", result.status_code);
                    println!("Links found: {}", result.links.len());
                    println!("Crawl time: {:?}", result.crawl_time);
                    println!("---");
                }
            }
        }
        Commands::ConcurrentCrawl {
            url,
            depth,
            concurrent,
            max_pages,
            external,
            output,
        } => {
            let config = CrawlerConfig {
                max_depth: depth,
                max_concurrent_requests: concurrent,
                max_pages,
                follow_external_links: external,
                delay_between_requests: Duration::from_millis(100),
                ..Default::default()
            };

            let crawler = WebCrawler::new(config)?;
            let results = crawler.crawl_concurrent(&url).await?;

            info!("Concurrent crawl completed! Foun
