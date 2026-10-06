package domain

import (
	"regexp"
	"sort"
	"strings"
)

var portReferencePattern = regexp.MustCompile(`\{[^{}\s]+\}`)

// PortReference is a service environment token. The final dot separates the
// service name from its named port; whitespace is not allowed inside tokens.
type PortReference struct {
	Token   string
	Service string
	Port    string
}

// Replace expands this token while preserving shell ${...} expressions.
func (r PortReference) Replace(value, replacement string) string {
	var out strings.Builder
	cursor := 0
	for _, loc := range portReferencePattern.FindAllStringIndex(value, -1) {
		if value[loc[0]:loc[1]] != r.Token || (loc[0] > 0 && value[loc[0]-1] == '$') {
			continue
		}
		out.WriteString(value[cursor:loc[0]])
		out.WriteString(replacement)
		cursor = loc[1]
	}
	out.WriteString(value[cursor:])
	return out.String()
}

// PortReferences returns unique references in lexical order. Shell ${...}
// expressions and brace text without a dot remain literal environment values.
func PortReferences(value string) []PortReference {
	refs := map[string]PortReference{}
	for _, loc := range portReferencePattern.FindAllStringIndex(value, -1) {
		if loc[0] > 0 && value[loc[0]-1] == '$' {
			continue
		}
		token := value[loc[0]:loc[1]]
		name := token[1 : len(token)-1]
		i := strings.LastIndexByte(name, '.')
		if i <= 0 || i == len(name)-1 {
			continue
		}
		refs[token] = PortReference{Token: token, Service: name[:i], Port: name[i+1:]}
	}
	return sortedPortReferences(refs)
}

// PortReferences returns the unique references declared in a service's env.
func (s Service) PortReferences() []PortReference {
	refs := map[string]PortReference{}
	for _, value := range s.Env {
		for _, ref := range PortReferences(value) {
			refs[ref.Token] = ref
		}
	}
	return sortedPortReferences(refs)
}

func sortedPortReferences(refs map[string]PortReference) []PortReference {
	out := make([]PortReference, 0, len(refs))
	for _, ref := range refs {
		out = append(out, ref)
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Token < out[j].Token })
	return out
}
