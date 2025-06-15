use dashmap::DashSet;
use futures::stream::{self, StreamExt};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};
use url::Url;

#[derive(Error, Debug)]
pub enum CrawlerError {
    #[error("HTTP request failed: {0}")]
    RequestError(#[from] reqwest::Error),
    #[error("URL parsing failed: {0}")]
    UrlParseError(#[from] url::ParseError),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Invalid selector: {0}")]
    SelectorError(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrawlResult {
    pub url: String,
    pub title: String,
    pub links: Vec<String>,
    pub status_code: u16,
    pub content_length: Option<u64>,
    pub crawl_time: Duration,
}

#[derive(Debug, Clone)]
pub struct CrawlerConfig {
    pub max_depth: usize,
    pub max_concurrent_requests: usize,
    pub delay_between_requests: Duration,
    pub timeout: Duration,
    pub follow_external_links: bool,
    pub user_agent: String,
    pub max_pages: Option<usize>,
}

impl Default for CrawlerConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_concurrent_requests: 10,
            delay_between_requests: Duration::from_millis(100),
            timeout: Duration::from_secs(30),
            follow_external_links: false,
            user_agent: "RustWebCrawler/1.0".to_string(),
            max_pages: Some(100),
        }
    }
}

pub struct WebCrawler {
    client: Client,
    config: CrawlerConfig,
    visited_urls: Arc<DashSet<String>>,
    semaphore: Arc<Semaphore>,
}

impl WebCrawler {
    pub fn new(config: CrawlerConfig) -> Result<Self, CrawlerError> {
        let client = Client::builder()
            .timeout(config.timeout)
            .user_agent(&config.user_agent)
            .build()?;

        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_requests));

        Ok(Self {
            client,
            config,
            visited_urls: Arc::new(DashSet::new()),
            semaphore,
        })
    }

    pub async fn crawl(&self, start_url: &str) -> Result<Vec<CrawlResult>, CrawlerError> {
        let start_url = Url::parse(start_url)?;
        let base_domain = start_url.domain().unwrap_or("").to_string();
        
        info!("Starting crawl from: {}", start_url);
        
        let mut results = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back((start_url.to_string(), 0));

        while let Some((url, depth)) = queue.pop_front() {
            if depth > self.config.max_depth {
                debug!("Max depth reached for URL: {}", url);
                continue;
            }

            if self.visited_urls.contains(&url) {
                debug!("URL already visited: {}", url);
                continue;
            }

            if let Some(max_pages) = self.config.max_pages {
                if results.len() >= max_pages {
                    info!("Max pages limit reached");
                    break;
                }
            }

            self.visited_urls.insert(url.clone());

            match self.crawl_page(&url).await {
                Ok(result) => {
                    info!("Successfully crawled: {} ({})", url, result.status_code);
                    
                    // Add new links to queue
                    for link in &result.links {
                        if let Ok(link_url) = Url::parse(link) {
                            let should_follow = if self.config.follow_external_links {
                                true
                            } else {
                                link_url.domain().unwrap_or("") == base_domain
                            };

                            if should_follow && !self.visited_urls.contains(link) {
                                queue.push_back((link.clone(), depth + 1));
                            }
                        }
                    }
                    
                    results.push(result);
                }
                Err(e) => {
                    error!("Failed to crawl {}: {}", url, e);
                }
            }

            // Respect delay between requests
            if !self.config.delay_between_requests.is_zero() {
                tokio::time::sleep(self.config.delay_between_requests).await;
            }
        }

        info!("Crawl completed. Total pages crawled: {}", results.len());
        Ok(results)
    }

    pub async fn crawl_concurrent(&self, start_url: &str) -> Result<Vec<CrawlResult>, CrawlerError> {
        let start_url = Url::parse(start_url)?;
        let base_domain = start_url.domain().unwrap_or("").to_string();
        
        info!("Starting concurrent crawl from: {}", start_url);
        
        let mut results = Vec::new();
        let mut queue = VecDeque::new();
        queue.push_back((start_url.to_string(), 0));

        while !queue.is_empty() {
            let current_batch: Vec<_> = queue
                .drain(..)
                .filter(|(url, depth)| {
                    *depth <= self.config.max_depth && !self.visited_urls.contains(url)
                })
                .take(self.config.max_concurrent_requests)
                .collect();

            if current_batch.is_empty() {
                break;
            }

            // Mark URLs as visited
            for (url, _) in &current_batch {
                self.visited_urls.insert(url.clone());
            }

            let batch_results = stream::iter(current_batch)
                .map(|(url, depth)| async move {
                    let _permit = self.semaphore.acquire().await.unwrap();
                    (self.crawl_page(&url).await, depth)
                })
                .buffer_unordered(self.config.max_concurrent_requests)
                .collect::<Vec<_>>()
                .await;

            for (result, depth) in batch_results {
                match result {
                    Ok(crawl_result) => {
                        info!("Successfully crawled: {} ({})", crawl_result.url, crawl_result.status_code);
                        
                        // Add new links to queue
                        for link in &crawl_result.links {
                            if let Ok(link_url) = Url::parse(link) {
                                let should_follow = if self.config.follow_external_links {
                                    true
                                } else {
                                    link_url.domain().unwrap_or("") == base_domain
                                };

                                if should_follow && !self.visited_urls.contains(link) {
                                    queue.push_back((link.clone(), depth + 1));
                                }
                            }
                        }
                        
                        results.push(crawl_result);
                    }
                    Err(e) => {
                        error!("Failed to crawl page: {}", e);
                    }
                }
            }

            if let Some(max_pages) = self.config.max_pages {
                if results.len() >= max_pages {
                    info!("Max pages limit reached");
                    break;
                }
            }
        }

        info!("Concurrent crawl completed. Total pages crawled: {}", results.len());
        Ok(results)
    }

    async fn crawl_page(&self, url: &str) -> Result<CrawlResult, CrawlerError> {
        let start_time = std::time::Instant::now();
        
        debug!("Crawling page: {}", url);
        
        let response = self.client.get(url).send().await?;
        let status_code = response.status().as_u16();
        let content_length = response.content_length();
        
        if !response.status().is_success() {
            warn!("Non-success status code {} for URL: {}", status_code, url);
        }

        let html = response.text().await?;
        let document = Html::parse_document(&html);

        // Extract title
        let title_selector = Selector::parse("title")
            .map_err(|e| CrawlerError::SelectorError(format!("title selector: {:?}", e)))?;
        let title = document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "No title".to_string());

        // Extract links
        let link_selector = Selector::parse("a[href]")
            .map_err(|e| CrawlerError::SelectorError(format!("link selector: {:?}", e)))?;
        
        let base_url = Url::parse(url)?;
        let mut links = Vec::new();

        for element in document.select(&link_selector) {
            if let Some(href) = element.value().attr("href") {
                match base_url.join(href) {
                    Ok(absolute_url) => {
                        let url_str = absolute_url.to_string();
                        if url_str.starts_with("http") {
                            links.push(url_str);
                        }
                    }
                    Err(e) => {
                        debug!("Failed to parse URL {}: {}", href, e);
                    }
                }
            }
        }

        // Remove duplicates
        links.sort();
        links.dedup();

        let crawl_time = start_time.elapsed();

        Ok(CrawlResult {
            url: url.to_string(),
            title,
            links,
            status_code,
            content_length,
            crawl_time,
        })
    }
}
