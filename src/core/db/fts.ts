function tokenize(run: string) {
  const points = Array.from(run, point => point.codePointAt(0)!.toString(16).padStart(6, '0'))
  return points.slice(0, -1).map((point, index) => 'h' + point + points[index + 1]).join(' ')
}

export function cjkIndex(content: string) {
  return Array.from(content.matchAll(/\p{Script=Han}+/gu), match => tokenize(match[0])).filter(Boolean).join(' gap ')
}

export function cjkQuery(query: string) {
  return /^(?:\p{Script=Han}){2,80}$/u.test(query) ? tokenize(query) : ''
}
