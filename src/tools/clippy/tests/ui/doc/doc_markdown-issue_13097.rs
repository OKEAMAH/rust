// This test checks that words starting with capital letters and ending with "ified" don't
// trigger the lint.

#![deny(clippy::doc_markdown)]

pub enum OutputFormat {
    /// HumaNified
    //~^ ERROR: item in documentation is missing backticks
    Plain,
    // Should not warn!
    /// JSONified console output
    Json,
}
