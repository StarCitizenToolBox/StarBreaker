//! UI snapshot comparison module split by responsibility.

include!("types.inc");
include!("capture.inc");
include!("compare.inc");
include!("metadata.inc");

#[cfg(test)]
mod tests_support;

#[cfg(test)]
include!("tests.inc");
