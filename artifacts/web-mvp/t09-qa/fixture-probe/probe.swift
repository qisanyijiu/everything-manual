import Foundation
import PDFKit

func probe(_ path: String) {
    guard let doc = PDFDocument(url: URL(fileURLWithPath: path)) else {
        print("\(path): 无法打开"); return
    }
    print("\(path): pageCount=\(doc.pageCount) isEncrypted=\(doc.isEncrypted) isLocked=\(doc.isLocked)")
    if doc.isLocked {
        let unlocked = doc.unlock(withPassword: "fixture-secret")
        print("  用 fixture-secret 解锁: \(unlocked)；解锁后 pageCount=\(doc.pageCount)")
    }
    if let first = doc.page(at: 0)?.string {
        let trimmed = first.trimmingCharacters(in: .whitespacesAndNewlines)
        print("  第 1 页文字前 60 字符: \(trimmed.prefix(60))")
    }
    if doc.pageCount >= 2, let second = doc.page(at: 1)?.string {
        let trimmed = second.trimmingCharacters(in: .whitespacesAndNewlines)
        print("  第 2 页文字前 60 字符: \(trimmed.prefix(60))")
    }
}

let root = CommandLine.arguments[1]
for name in ["sample-manual-text.pdf", "sample-manual-scan.pdf", "sample-manual-rotated.pdf", "sample-manual-nonlatin.pdf", "sample-manual-encrypted.pdf", "sample-manual-many-pages.pdf"] {
    probe("\(root)/\(name)")
}
