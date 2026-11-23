use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::Path;

use maxminddb::{MaxMindDbError, Mmap, Reader};
use serde::Deserialize;

/// Where an address is and who runs its network, from DB-IP Lite.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Place {
    /// ISO 3166-1 alpha-2, e.g. `BE`.
    pub country: Option<String>,
    pub city: Option<String>,
    pub asn: Option<u32>,
    /// The AS organization, e.g. `Google LLC`.
    pub network: Option<String>,
}

/// DB-IP City Lite and ASN Lite, memory-mapped. Either may be missing.
#[derive(Default)]
pub struct Geo {
    city: Option<Reader<Mmap>>,
    asn: Option<Reader<Mmap>>,
}

#[derive(Deserialize)]
struct CityRecord<'a> {
    #[serde(borrow)]
    city: Option<Names<'a>>,
    #[serde(borrow)]
    country: Option<Country<'a>>,
}

#[derive(Deserialize)]
struct Names<'a> {
    #[serde(borrow)]
    names: BTreeMap<&'a str, &'a str>,
}

#[derive(Deserialize)]
struct Country<'a> {
    iso_code: Option<&'a str>,
}

#[derive(Deserialize)]
struct AsnRecord<'a> {
    autonomous_system_number: Option<u32>,
    autonomous_system_organization: Option<&'a str>,
}

impl CityRecord<'_> {
    fn fill(&self, place: &mut Place) {
        place.country = self
            .country
            .as_ref()
            .and_then(|c| c.iso_code)
            .map(str::to_owned);
        place.city = self
            .city
            .as_ref()
            .and_then(|c| c.names.get("en"))
            .map(|&s| s.to_owned());
    }
}

impl AsnRecord<'_> {
    fn fill(&self, place: &mut Place) {
        place.asn = self.autonomous_system_number;
        place.network = self.autonomous_system_organization.map(str::to_owned);
    }
}

/// Memory-maps one database file.
///
/// # Errors
/// When the file is missing or not a valid MMDB.
pub fn open(path: &Path) -> Result<Reader<Mmap>, MaxMindDbError> {
    // SAFETY: the service never rewrites a database in place: each month's file gets a new
    // name, and an old one is only deleted after it is unmapped.
    unsafe { Reader::open_mmap(path) }
}

impl Geo {
    #[must_use]
    pub fn new(city: Option<Reader<Mmap>>, asn: Option<Reader<Mmap>>) -> Self {
        Self { city, asn }
    }

    /// Everything known about `ip`; empty for private addresses or missing databases.
    #[must_use]
    pub fn place(&self, ip: IpAddr) -> Place {
        let mut place = Place::default();
        if let Some(r) = &self.city
            && let Ok(found) = r.lookup(ip)
            && let Ok(Some(rec)) = found.decode::<CityRecord>()
        {
            rec.fill(&mut place);
        }
        if let Some(r) = &self.asn
            && let Ok(found) = r.lookup(ip)
            && let Ok(Some(rec)) = found.decode::<AsnRecord>()
        {
            rec.fill(&mut place);
        }
        place
    }
}

/// An AS organization as a short service name: `Fastly, Inc.` → `Fastly`.
#[must_use]
pub fn org_name(org: &str) -> String {
    const SUFFIXES: [&str; 16] = [
        "inc",
        "llc",
        "ltd",
        "limited",
        "corporation",
        "corp",
        "co",
        "gmbh",
        "ag",
        "bv",
        "b.v",
        "sa",
        "s.a",
        "sas",
        "pbc",
        "plc",
    ];
    let mut name = org.trim();
    loop {
        let trimmed = name.trim_end_matches(['.', ',', ' ']);
        let Some((head, last)) = trimmed.rsplit_once([' ', ',']) else {
            return trimmed.to_owned();
        };
        if !SUFFIXES.contains(&last.to_ascii_lowercase().as_str()) || head.trim().is_empty() {
            return trimmed.to_owned();
        }
        name = head;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn city_records_map_to_a_place() {
        let rec: CityRecord = serde_json::from_str(
            r#"{"city":{"names":{"en":"Saint-Ghislain","fr":"Saint-Ghislain"}},
                "country":{"iso_code":"BE","names":{"en":"Belgium"}},
                "location":{"latitude":50.45,"longitude":3.82}}"#,
        )
        .unwrap();
        let mut place = Place::default();
        rec.fill(&mut place);
        assert_eq!(place.country.as_deref(), Some("BE"));
        assert_eq!(place.city.as_deref(), Some("Saint-Ghislain"));
    }

    #[test]
    fn asn_records_map_to_a_place() {
        let rec: AsnRecord = serde_json::from_str(
            r#"{"autonomous_system_number":15169,"autonomous_system_organization":"Google LLC"}"#,
        )
        .unwrap();
        let mut place = Place::default();
        rec.fill(&mut place);
        assert_eq!(
            (place.asn, place.network.as_deref()),
            (Some(15169), Some("Google LLC"))
        );
    }

    #[test]
    fn missing_databases_give_an_empty_place() {
        assert_eq!(
            Geo::default().place("8.8.8.8".parse().unwrap()),
            Place::default()
        );
    }

    #[test]
    fn org_names_drop_legal_suffixes() {
        for (org, name) in [
            ("Fastly, Inc.", "Fastly"),
            ("Google LLC", "Google"),
            ("Microsoft Corporation", "Microsoft"),
            ("Anthropic, PBC", "Anthropic"),
            ("Hetzner Online GmbH", "Hetzner Online"),
            ("Amazon.com, Inc.", "Amazon.com"),
            ("Cloudflare, Inc.", "Cloudflare"),
            ("Telenet BV", "Telenet"),
            ("LLC", "LLC"),
            ("Proximus", "Proximus"),
        ] {
            assert_eq!(org_name(org), name, "{org}");
        }
    }

    /// Run with real files: `PARCEL_DATA=<dir> cargo test -p parcel-core -- --ignored`.
    #[test]
    #[ignore = "needs the DB-IP files in PARCEL_DATA"]
    fn real_databases_answer() {
        let dir = std::path::PathBuf::from(std::env::var("PARCEL_DATA").unwrap());
        let newest = |kind: &str| {
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    let n = p.file_name().unwrap().to_string_lossy().into_owned();
                    n.starts_with(&format!("dbip-{kind}-lite-")) && n.ends_with(".mmdb")
                })
                .collect();
            files.sort();
            open(files.last().expect("no database")).unwrap()
        };
        let geo = Geo::new(Some(newest("city")), Some(newest("asn")));
        let place = geo.place("8.8.8.8".parse().unwrap());
        assert_eq!(place.country.as_deref(), Some("US"), "{place:?}");
        assert_eq!(place.asn, Some(15169), "{place:?}");
        assert!(place.network.is_some_and(|n| n.contains("Google")));
        assert_eq!(geo.place("192.168.1.1".parse().unwrap()).asn, None);
    }
}
