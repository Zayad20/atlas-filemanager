//! vCard 3/4 contact fields, UTF-8 unfolding/escaping and RFC 4180 tables.
//! Photos, custom properties and per-value labels remain in the source vCard.
use super::{native, Command};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::Write;
use std::path::Path;

const HEADERS: &[&str] = &[
    "name",
    "family_name",
    "given_name",
    "additional_names",
    "prefix",
    "suffix",
    "emails",
    "phones",
    "organization",
    "title",
    "urls",
    "birthday",
    "note",
    "addresses",
];

fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct StringOrVec;
    impl<'de> serde::de::Visitor<'de> for StringOrVec {
        type Value = Vec<String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or list of strings")
        }
        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(if value.is_empty() {
                Vec::new()
            } else {
                vec![value.to_owned()]
            })
        }
        fn visit_seq<S>(self, mut visitor: S) -> Result<Self::Value, S::Error>
        where
            S: serde::de::SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(elem) = visitor.next_element()? {
                vec.push(elem);
            }
            Ok(vec)
        }
    }
    deserializer.deserialize_any(StringOrVec)
}

#[derive(Default, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct Contact {
    name: String,
    family_name: String,
    given_name: String,
    additional_names: String,
    prefix: String,
    suffix: String,
    #[serde(deserialize_with = "string_or_vec")]
    emails: Vec<String>,
    #[serde(deserialize_with = "string_or_vec")]
    phones: Vec<String>,
    organization: String,
    title: String,
    #[serde(deserialize_with = "string_or_vec")]
    urls: Vec<String>,
    birthday: String,
    note: String,
    /// RFC 6350 address components: PO box, extended, street, city, region, code, country.
    addresses: Vec<[String; 7]>,
}

fn escape(value: &str) -> Result<String> {
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\r' | '\n' | '\t')),
        "Contact contains unsupported control characters"
    );
    Ok(value
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,"))
}

fn unescape(value: &str) -> Result<String> {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            result.push(match chars.next().context("Unterminated vCard escape")? {
                'n' | 'N' => '\n',
                c @ ('\\' | ',' | ';' | ':') => c,
                _ => bail!("Unknown vCard text escape"),
            });
        } else {
            result.push(c);
        }
    }
    Ok(result)
}

fn components(value: &str, count: usize) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut escaped = false;
    for (index, c) in value.char_indices() {
        if c == ';' && !escaped {
            parts.push(unescape(&value[start..index])?);
            start = index + 1;
        }
        escaped = c == '\\' && !escaped;
    }
    parts.push(unescape(&value[start..])?);
    ensure!(
        parts.len() <= count,
        "Too many components in a structured vCard field"
    );
    parts.resize(count, String::new());
    Ok(parts)
}

fn vcard(text: &str) -> Result<Vec<Contact>> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        ensure!(
            lines.len() < 1_000_000,
            "vCard exceeds one million content lines"
        );
        if line.starts_with([' ', '\t']) {
            lines
                .last_mut()
                .context("vCard starts with a folded continuation")?
                .push_str(&line[1..]);
        } else {
            lines.push(line.to_owned());
        }
    }
    let mut contacts = Vec::new();
    let mut current: Option<Contact> = None;
    let mut version = false;
    for line in lines.iter().filter(|line| !line.is_empty()) {
        let mut quoted = false;
        let split = line
            .char_indices()
            .find_map(|(index, c)| {
                if c == '"' {
                    quoted = !quoted;
                }
                (c == ':' && !quoted).then_some(index)
            })
            .context("vCard property has no colon")?;
        let (property, value) = (&line[..split], &line[split + 1..]);
        let key = property
            .split(';')
            .next()
            .unwrap_or_default()
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        match key.as_str() {
            "BEGIN" => {
                ensure!(
                    value.eq_ignore_ascii_case("VCARD") && current.is_none(),
                    "Invalid nested vCard"
                );
                current = Some(Contact::default());
                version = false;
            }
            "END" => {
                ensure!(
                    value.eq_ignore_ascii_case("VCARD") && version,
                    "vCard needs VERSION:3.0 or VERSION:4.0"
                );
                let mut contact = current.take().context("vCard END without BEGIN")?;
                if contact.name.is_empty() {
                    contact.name = [
                        &contact.prefix,
                        &contact.given_name,
                        &contact.additional_names,
                        &contact.family_name,
                        &contact.suffix,
                    ]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(" ");
                }
                contacts.push(contact);
                ensure!(
                    contacts.len() <= 100_000,
                    "Contact input exceeds 100,000 records"
                );
            }
            "VERSION" => {
                ensure!(
                    current.is_some() && !version && ["3.0", "4.0"].contains(&value),
                    "Only UTF-8 vCard 3.0/4.0 is supported natively"
                );
                version = true;
            }
            _ => {
                let contact = current
                    .as_mut()
                    .context("vCard property outside BEGIN/END")?;
                let supported = [
                    "FN", "N", "EMAIL", "TEL", "ORG", "TITLE", "URL", "BDAY", "NOTE", "ADR",
                ]
                .contains(&key.as_str());
                if !supported {
                    continue;
                }
                for parameter in property.split(';').skip(1) {
                    let p = parameter.to_ascii_uppercase();
                    ensure!(
                        !p.starts_with("ENCODING=") || p == "ENCODING=8BIT",
                        "Encoded contact fields must be converted to UTF-8 before export"
                    );
                    ensure!(
                        !p.starts_with("CHARSET=") || p == "CHARSET=UTF-8",
                        "Expected UTF-8 contact fields"
                    );
                }
                let decoded = unescape(value)?;
                match key.as_str() {
                    "FN" => contact.name = decoded,
                    "N" => {
                        let parts = components(value, 5)?;
                        [
                            contact.family_name,
                            contact.given_name,
                            contact.additional_names,
                            contact.prefix,
                            contact.suffix,
                        ] = parts
                            .try_into()
                            .map_err(|_| anyhow::anyhow!("Invalid contact name"))?;
                    }
                    "EMAIL" => contact.emails.push(decoded),
                    "TEL" => contact
                        .phones
                        .push(decoded.strip_prefix("tel:").unwrap_or(&decoded).to_owned()),
                    "ORG" => {
                        contact.organization = components(value, 16)?
                            .into_iter()
                            .filter(|s| !s.is_empty())
                            .collect::<Vec<_>>()
                            .join(" / ")
                    }
                    "TITLE" => contact.title = decoded,
                    "URL" => contact.urls.push(decoded),
                    "BDAY" => contact.birthday = decoded,
                    "NOTE" => {
                        if !contact.note.is_empty() {
                            contact.note.push('\n');
                        }
                        contact.note.push_str(&decoded);
                    }
                    "ADR" => contact.addresses.push(
                        components(value, 7)?
                            .try_into()
                            .map_err(|_| anyhow::anyhow!("Invalid contact address"))?,
                    ),
                    _ => {}
                }
            }
        }
    }
    ensure!(
        current.is_none() && !contacts.is_empty(),
        "Unclosed or empty vCard document"
    );
    Ok(contacts)
}

fn folded(writer: &mut impl Write, line: &str) -> Result<()> {
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > 75 {
            writer.write_all(b"\r\n ")?;
            width = 1;
        }
        write!(writer, "{c}")?;
        width += c.len_utf8();
    }
    writer.write_all(b"\r\n")?;
    Ok(())
}

fn write_vcard(contacts: &[Contact], writer: &mut impl Write) -> Result<()> {
    for contact in contacts {
        let name = if contact.name.is_empty() {
            [&contact.given_name, &contact.family_name]
                .into_iter()
                .filter(|s| !s.is_empty())
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            contact.name.clone()
        };
        let name = if name.is_empty() {
            contact
                .emails
                .first()
                .or_else(|| contact.phones.first())
                .context("Contact needs a name, email or phone")?
                .clone()
        } else {
            name
        };
        writer.write_all(b"BEGIN:VCARD\r\nVERSION:3.0\r\n")?;
        folded(writer, &format!("FN:{}", escape(&name)?))?;
        let structured = [
            &contact.family_name,
            &contact.given_name,
            &contact.additional_names,
            &contact.prefix,
            &contact.suffix,
        ]
        .into_iter()
        .map(|s| escape(s))
        .collect::<Result<Vec<_>>>()?
        .join(";");
        folded(writer, &format!("N:{structured}"))?;
        for (key, value) in [
            ("ORG", &contact.organization),
            ("TITLE", &contact.title),
            ("BDAY", &contact.birthday),
            ("NOTE", &contact.note),
        ] {
            if !value.is_empty() {
                folded(writer, &format!("{key}:{}", escape(value)?))?;
            }
        }
        for (key, values) in [
            ("EMAIL", &contact.emails),
            ("TEL", &contact.phones),
            ("URL", &contact.urls),
        ] {
            for value in values {
                folded(writer, &format!("{key}:{}", escape(value)?))?;
            }
        }
        for address in &contact.addresses {
            let value = address
                .iter()
                .map(|s| escape(s))
                .collect::<Result<Vec<_>>>()?
                .join(";");
            folded(writer, &format!("ADR:{value}"))?;
        }
        writer.write_all(b"END:VCARD\r\n")?;
    }
    Ok(())
}

fn table(text: &str, delimiter: char) -> Result<Vec<Contact>> {
    let mut rows = native::parse_delimited(text, delimiter)?.into_iter();
    let headers = rows.next().context("Contact table is empty")?;
    let headers: Vec<_> = headers
        .iter()
        .map(|h| {
            let key = h.to_ascii_lowercase().replace([' ', '_', '-'], "");
            match key.as_str() {
                "name" | "fullname" | "displayname" => "name",
                "firstname" | "givenname" => "given_name",
                "lastname" | "familyname" => "family_name",
                "additionalnames" | "middlename" => "additional_names",
                "email" | "emails" => "emails",
                "phone" | "phones" | "telephone" => "phones",
                "url" | "urls" => "urls",
                "note" | "notes" => "note",
                "address" | "addresses" => "addresses",
                "organization" | "company" => "organization",
                "birthday" | "bday" => "birthday",
                "prefix" => "prefix",
                "suffix" => "suffix",
                "title" => "title",
                _ => "",
            }
        })
        .collect();
    ensure!(
        headers
            .iter()
            .any(|h| ["name", "given_name", "family_name", "emails", "phones"].contains(h)),
        "Contact table needs name, email or phone columns"
    );
    let known: Vec<_> = headers.iter().filter(|h| !h.is_empty()).collect();
    ensure!(
        known
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == known.len(),
        "Contact table has duplicate mapped columns"
    );
    let mut contacts = Vec::new();
    for row in rows {
        ensure!(
            row.len() == headers.len(),
            "Contact table row has the wrong number of columns"
        );
        let mut object = serde_json::Map::new();
        for (key, cell) in headers.iter().zip(row) {
            if key.is_empty() {
                continue;
            }
            let value = if ["emails", "phones", "urls", "addresses"].contains(key) {
                if cell.trim().is_empty() {
                    serde_json::json!([])
                } else if cell.trim_start().starts_with('[') {
                    serde_json::from_str(&cell)
                        .context("Contact array cell must contain valid JSON")?
                } else {
                    ensure!(
                        *key != "addresses",
                        "Address cells must contain a JSON array of seven-component arrays"
                    );
                    serde_json::json!([cell])
                }
            } else {
                serde_json::Value::String(cell)
            };
            object.insert((*key).to_owned(), value);
        }
        contacts.push(serde_json::from_value(serde_json::Value::Object(object))?);
        ensure!(
            contacts.len() <= 100_000,
            "Contact input exceeds 100,000 records"
        );
    }
    Ok(contacts)
}

pub(super) fn convert(command: &Command, output: &Path) -> Result<()> {
    let text = native::read_text(&command.source)?;
    let contacts = match super::catalog::extension(&command.source).as_str() {
        "vcf" => vcard(&text)?,
        "csv" => table(&text, ',')?,
        "tsv" => table(&text, '\t')?,
        "json" => serde_json::from_str::<Vec<Contact>>(&text)?,
        _ => bail!("Unsupported contact input"),
    };
    ensure!(
        !contacts.is_empty() && contacts.len() <= 100_000,
        "Expected 1–100000 contacts"
    );
    let mut file = File::create(output)?;
    match command.target.extension {
        "vcf" => write_vcard(&contacts, &mut file)?,
        "json" => serde_json::to_writer_pretty(file, &contacts)?,
        "csv" | "tsv" => {
            let delimiter = if command.target.extension == "csv" {
                ','
            } else {
                '\t'
            };
            native::write_row(
                &mut file,
                &HEADERS.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
                delimiter,
            )?;
            for contact in &contacts {
                let object = serde_json::to_value(contact)?;
                let row = HEADERS
                    .iter()
                    .map(|h| match &object[*h] {
                        serde_json::Value::String(s) => s.clone(),
                        value => value.to_string(),
                    })
                    .collect::<Vec<_>>();
                native::write_row(&mut file, &row, delimiter)?;
            }
        }
        _ => bail!("Unsupported contact output"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcard_unfolds_unicode_and_preserves_escaped_structured_components() {
        let text = "BEGIN:VCARD\r\nVERSION:4.0\r\nFN:María\r\n  del Mar\r\nN:Pérez\\;Soto;María;;;\r\nitem1.EMAIL;TYPE=work:ana@example.com\r\nTEL;VALUE=uri:tel:+521234\r\nNOTE:Dos\\nLíneas\\, literal\\; fin\r\nADR:;;Calle\\; 1;Ciudad;Región;123;México\r\nEND:VCARD\r\n";
        let contacts = vcard(text).unwrap();
        assert_eq!(contacts[0].name, "María del Mar");
        assert_eq!(contacts[0].family_name, "Pérez;Soto");
        assert_eq!(contacts[0].phones, ["+521234"]);
        assert_eq!(contacts[0].note, "Dos\nLíneas, literal; fin");
        assert_eq!(contacts[0].addresses[0][2], "Calle; 1");
        let mut bytes = Vec::new();
        write_vcard(&contacts, &mut bytes).unwrap();
        assert_eq!(
            vcard(std::str::from_utf8(&bytes).unwrap()).unwrap(),
            contacts
        );
    }

    #[test]
    fn output_folds_at_75_octets_without_splitting_utf8_or_injecting_properties() {
        let contact = Contact {
            name: "ñ".repeat(80),
            note: "hello\r\nTEL:injected".into(),
            ..Contact::default()
        };
        let mut bytes = Vec::new();
        write_vcard(&[contact], &mut bytes).unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(text.lines().all(|line| line.len() <= 75));
        assert!(!text.lines().any(|line| line == "TEL:injected"));
        assert_eq!(vcard(text).unwrap()[0].name, "ñ".repeat(80));
        assert_eq!(vcard(text).unwrap()[0].note, "hello\nTEL:injected");
    }
}
