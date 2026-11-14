package bridge

import (
	"bytes"
	"encoding/xml"
	"fmt"
	"io"
)

// qidiStudioApplication is the exact importer-compatibility value verified
// against the owner's native QIDIStudio GUI A/B test. It affects only the
// 3MF model's project-origin metadata, never its material/profile settings.
const qidiStudioApplication = "QIDIStudio-01.09.03.51"

var qidiApplicationElement = []byte("<metadata name=\"Application\">" + qidiStudioApplication + "</metadata>")
var qidiApplicationInsertion = append([]byte("\n  "), qidiApplicationElement...)

// setQIDIApplicationMetadata rewrites exactly one model-level XML metadata
// field. The input XML is fully parsed before any bytes are changed; the
// remaining model (including every triangle, transform and material) is copied
// byte-for-byte. Existing project Application metadata is replaced, not
// duplicated. The conversion runs this only for an actual QIDI target.
func setQIDIApplicationMetadata(raw []byte) ([]byte, error) {
	dec := xml.NewDecoder(bytes.NewReader(raw))
	depth := 0
	rootEnd := -1
	rootPrefix := ""
	applicationStart, applicationEnd, openApplication := -1, -1, -1
	for {
		from := int(dec.InputOffset())
		tok, err := dec.Token()
		to := int(dec.InputOffset())
		if err == io.EOF {
			break
		}
		if err != nil {
			return nil, fmt.Errorf("QIDI model XML: %w", err)
		}
		switch t := tok.(type) {
		case xml.StartElement:
			if depth == 0 {
				if t.Name.Local != "model" || rootEnd >= 0 {
					return nil, fmt.Errorf("QIDI 3MF has no single model XML root")
				}
				if bytes.HasSuffix(bytes.TrimSpace(raw[from:to]), []byte("/>")) {
					return nil, fmt.Errorf("self-closing 3MF model root cannot contain Application metadata")
				}
				opening := bytes.TrimSpace(raw[from:to])
				if len(opening) < 2 || opening[0] != '<' {
					return nil, fmt.Errorf("malformed model XML start tag")
				}
				qname := opening[1:]
				endName := bytes.IndexAny(qname, " \t\r\n/>")
				if endName < 0 {
					return nil, fmt.Errorf("malformed model XML qualified name")
				}
				if colon := bytes.IndexByte(qname[:endName], ':'); colon >= 0 {
					rootPrefix = string(qname[:colon+1])
				}
				rootEnd = to
			} else if depth == 1 && t.Name.Local == "metadata" {
				for _, a := range t.Attr {
					if a.Name.Local == "name" && a.Value == "Application" {
						if applicationStart >= 0 || openApplication >= 0 {
							return nil, fmt.Errorf("duplicate 3MF Application metadata")
						}
						openApplication = from
					}
				}
			}
			depth++
		case xml.EndElement:
			if depth == 2 && t.Name.Local == "metadata" && openApplication >= 0 {
				applicationStart, applicationEnd = openApplication, to
				openApplication = -1
			}
			depth--
		case xml.Directive:
			return nil, fmt.Errorf("3MF model XML directives are not permitted")
		}
	}
	if rootEnd <= 0 || depth != 0 || openApplication >= 0 {
		return nil, fmt.Errorf("incomplete QIDI 3MF model XML")
	}
	element := qidiApplicationElement
	insertion := qidiApplicationInsertion
	if rootPrefix != "" {
		element = []byte("<" + rootPrefix + "metadata name=\"Application\">" + qidiStudioApplication + "</" + rootPrefix + "metadata>")
		insertion = append([]byte("\n  "), element...)
	}
	if applicationStart >= 0 {
		// Keep a namespace-qualified metadata element in the root's namespace.
		out := make([]byte, 0, len(raw)+len(element))
		out = append(out, raw[:applicationStart]...)
		out = append(out, element...)
		out = append(out, raw[applicationEnd:]...)
		return out, nil
	}
	out := make([]byte, 0, len(raw)+len(insertion))
	out = append(out, raw[:rootEnd]...)
	out = append(out, insertion...)
	out = append(out, raw[rootEnd:]...)
	return out, nil
}
