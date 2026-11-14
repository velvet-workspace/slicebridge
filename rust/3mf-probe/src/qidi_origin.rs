//! QIDIStudio importer metadata compatibility applied automatically only
//! to QIDI conversion outputs. Changes only model-level Application XML.
use quick_xml::events::Event;
use quick_xml::Reader;

const QIDI_APPLICATION: &[u8] =
    b"<metadata name=\"Application\">QIDIStudio-01.09.03.51</metadata>";
const QIDI_INSERTION: &[u8] =
    b"\n  <metadata name=\"Application\">QIDIStudio-01.09.03.51</metadata>";

pub(crate) fn set_qidi_application_metadata(raw: &[u8]) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(raw);
    let mut depth = 0usize;
    let mut root_end: Option<usize> = None;
    let mut app_start: Option<usize> = None;
    let mut app_end: Option<usize> = None;
    let mut opened_app: Option<usize> = None;
    loop {
        let before = reader.buffer_position() as usize;
        let event = reader
            .read_event()
            .map_err(|e| format!("QIDI model XML: {e}"))?;
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(start) => {
                if depth == 0 {
                    if start.local_name().as_ref() != "model" || root_end.is_some() {
                        return Err("QIDI 3MF has no single model XML root".into());
                    }
                    root_end = Some(after);
                } else if depth == 1 && start.local_name().as_ref() == "metadata" {
                    for attribute in start.attributes() {
                        let attribute = attribute.map_err(|e| format!("3MF attribute: {e}"))?;
                        if attribute.key.local_name().as_ref() == "name"
                            && attribute.value.as_ref() == "Application"
                        {
                            if app_start.is_some() || opened_app.is_some() {
                                return Err("duplicate 3MF Application metadata".into());
                            }
                            opened_app = Some(before);
                        }
                    }
                }
                depth += 1;
            }
            Event::Empty(start) => {
                if depth == 1 && start.local_name().as_ref() == "metadata" {
                    for attribute in start.attributes() {
                        let attribute = attribute.map_err(|e| format!("3MF attribute: {e}"))?;
                        if attribute.key.local_name().as_ref() == "name"
                            && attribute.value.as_ref() == "Application"
                        {
                            if app_start.is_some() || opened_app.is_some() {
                                return Err("duplicate 3MF Application metadata".into());
                            }
                            app_start = Some(before);
                            app_end = Some(after);
                        }
                    }
                }
            }
            Event::End(end) => {
                if depth == 0 {
                    return Err("unexpected 3MF model XML end tag".into());
                }
                if depth == 2 && end.local_name().as_ref() == "metadata" {
                    if let Some(first) = opened_app.take() {
                        app_start = Some(first);
                        app_end = Some(after);
                    }
                }
                depth -= 1;
            }
            Event::DocType(_) => return Err("3MF model XML DOCTYPE is not permitted".into()),
            Event::Eof => break,
            _ => {}
        }
    }
    let root_end = root_end.ok_or_else(|| "missing 3MF model root".to_owned())?;
    if depth != 0 || opened_app.is_some() {
        return Err("incomplete QIDI 3MF model XML".into());
    }
    let (from, to) = match (app_start, app_end) {
        (Some(from), Some(to)) => (from, to),
        (None, None) => (root_end, root_end),
        _ => return Err("incomplete 3MF Application metadata".into()),
    };
    let mut result = Vec::with_capacity(raw.len() + QIDI_APPLICATION.len());
    result.extend_from_slice(&raw[..from]);
    if from == to {
        result.extend_from_slice(QIDI_INSERTION);
    } else {
        result.extend_from_slice(QIDI_APPLICATION);
    }
    result.extend_from_slice(&raw[to..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adds_and_replaces_only_model_application() {
        let source = b"<?xml version='1.0'?><model xmlns='urn:3mf'><resources/><build/></model>";
        let converted = set_qidi_application_metadata(source).unwrap();
        assert_eq!(converted.iter().filter(|b| **b==b'<').count(), source.iter().filter(|b| **b==b'<').count()+2);
        assert!(converted.windows(QIDI_APPLICATION.len()).any(|w| w==QIDI_APPLICATION));
        assert_eq!(set_qidi_application_metadata(&converted).unwrap(), converted);
        let old = b"<model><metadata name='Application'>BambuStudio-1.0</metadata><build/></model>";
        let changed = set_qidi_application_metadata(old).unwrap();
        assert!(!changed.windows(b"BambuStudio".len()).any(|w| w==b"BambuStudio"));
        assert_eq!(set_qidi_application_metadata(&changed).unwrap(), changed);
        assert!(set_qidi_application_metadata(b"<model><metadata name='Application'>A</metadata><metadata name='Application'>B</metadata></model>").is_err());
    }
}
