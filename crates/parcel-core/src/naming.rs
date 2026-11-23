use std::collections::{BTreeMap, HashMap};

/// Hand-curated: brand → the registered domains it serves from.
const BRANDS: &str = include_str!("../brands.json");

/// What a hostname is shown as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Named {
    /// Lowercase, without a trailing dot.
    pub host: String,
    /// Registered domain (`googlevideo.com`), for the favicon.
    pub domain: Option<String>,
    /// The brand (`YouTube`), else the registered domain.
    pub service: Option<String>,
}

/// Names remotes: hostname → registered domain (public suffix list) → brand table.
pub struct Naming {
    brands: HashMap<String, String>,
}

impl Default for Naming {
    fn default() -> Self {
        let table: BTreeMap<String, Vec<String>> =
            serde_json::from_str(BRANDS).expect("brands.json is valid (checked by tests)");
        let brands = table
            .into_iter()
            .flat_map(|(brand, domains)| domains.into_iter().map(move |d| (d, brand.clone())))
            .collect();
        Self { brands }
    }
}

impl Naming {
    #[must_use]
    pub fn name(&self, hostname: &str) -> Named {
        let host = hostname.trim_end_matches('.').to_ascii_lowercase();
        let domain = psl::domain_str(&host).map(str::to_owned);
        // Hosting suffixes (`github.io`, `cloudfront.net`) make every customer its own domain;
        // the brand then sits on the suffix.
        let brand = domain
            .as_deref()
            .and_then(|d| self.brands.get(d))
            .or_else(|| psl::suffix_str(&host).and_then(|s| self.brands.get(s)));
        let service = brand.or(domain.as_ref()).cloned();
        Named {
            host,
            domain,
            service,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn service(host: &str) -> Option<String> {
        Naming::default().name(host).service
    }

    #[test]
    fn brands_name_their_domains() {
        assert_eq!(
            service("rr2---sn-u125g5-5w.googlevideo.com").as_deref(),
            Some("YouTube")
        );
        assert_eq!(
            service("github.githubassets.com").as_deref(),
            Some("GitHub")
        );
        assert_eq!(service("api.anthropic.com").as_deref(), Some("Anthropic"));
    }

    #[test]
    fn brands_on_hosting_suffixes_name_their_customers() {
        let named = Naming::default().name("someone.github.io");
        assert_eq!(named.domain.as_deref(), Some("someone.github.io"));
        assert_eq!(named.service.as_deref(), Some("GitHub"));
        assert_eq!(service("a1.akadns.net").as_deref(), Some("Akamai"));
    }

    #[test]
    fn unbranded_hosts_fall_back_to_the_registered_domain() {
        let named = Naming::default().name("Cdn.Example.CO.UK.");
        assert_eq!(named.host, "cdn.example.co.uk");
        assert_eq!(named.domain.as_deref(), Some("example.co.uk"));
        assert_eq!(named.service.as_deref(), Some("example.co.uk"));
    }

    #[test]
    fn single_labels_have_no_domain() {
        assert_eq!(Naming::default().name("localhost").domain, None);
    }

    #[test]
    fn brand_table_is_well_formed() {
        let table: BTreeMap<String, Vec<String>> = serde_json::from_str(BRANDS).unwrap();
        let mut seen = HashSet::new();
        for (brand, domains) in &table {
            assert!(!brand.trim().is_empty());
            assert!(!domains.is_empty(), "{brand} has no domains");
            for d in domains {
                assert!(seen.insert(d.as_str()), "{d} is listed twice");
                assert!(
                    psl::domain_str(d) == Some(d.as_str())
                        || psl::suffix_str(d) == Some(d.as_str()),
                    "{d} ({brand}) must be a registered domain or a public suffix, lowercase"
                );
            }
        }
        assert!(seen.len() >= 200, "only {} domains", seen.len());
    }
}
