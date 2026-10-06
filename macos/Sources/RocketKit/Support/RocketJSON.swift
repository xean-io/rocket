import Foundation

/// JSON coding matching Go's `encoding/json`: snake_case keys and RFC 3339
/// timestamps with optional nanosecond fractions.
public enum RocketJSON {
    public static var decoder: JSONDecoder {
        let d = JSONDecoder()
        d.keyDecodingStrategy = .convertFromSnakeCase
        d.dateDecodingStrategy = .custom { decoder in
            let container = try decoder.singleValueContainer()
            let raw = try container.decode(String.self)
            guard let date = parseDate(raw) else {
                throw DecodingError.dataCorruptedError(in: container, debugDescription: "invalid RFC 3339 date \(raw)")
            }
            return date
        }
        return d
    }

    public static var encoder: JSONEncoder {
        let e = JSONEncoder()
        e.keyEncodingStrategy = .convertToSnakeCase
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.sortedKeys]
        return e
    }

    /// Parses Go's RFC 3339 (nano) output. Fractions of any length are kept
    /// (Foundation's ISO 8601 parser only understands milliseconds reliably).
    public static func parseDate(_ raw: String) -> Date? {
        var base = raw
        var fraction = 0.0
        if let dot = raw.firstIndex(of: ".") {
            let digits = raw[raw.index(after: dot)...].prefix { $0.isASCII && $0.isNumber }
            guard !digits.isEmpty else { return nil }
            fraction = Double("0." + digits) ?? 0
            base = String(raw[..<dot]) + String(raw[digits.endIndex...])
        }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime]
        guard let date = formatter.date(from: base) else { return nil }
        return date.addingTimeInterval(fraction)
    }
}

/// A string-backed enum that tolerates values added by newer daemons.
public protocol OpenEnum: RawRepresentable, Codable, Hashable, Sendable, CustomStringConvertible
where RawValue == String {
    init(rawValue: String)
}

extension OpenEnum {
    public init(from decoder: Decoder) throws {
        self.init(rawValue: try decoder.singleValueContainer().decode(String.self))
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }

    public var description: String { rawValue }
}
