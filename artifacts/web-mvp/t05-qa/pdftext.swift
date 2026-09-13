import Foundation
import PDFKit

let args = CommandLine.arguments
for path in args.dropFirst() {
    guard let doc = PDFDocument(url: URL(fileURLWithPath: path)) else {
        print("\(path): 无法打开")
        continue
    }
    var text = ""
    for i in 0..<doc.pageCount {
        if let page = doc.page(at: i), let t = page.string { text += t }
    }
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    print("\(path): pages=\(doc.pageCount) textChars=\(trimmed.count) preview=\(trimmed.prefix(60).replacingOccurrences(of: "\n", with: "\\n"))")
}
