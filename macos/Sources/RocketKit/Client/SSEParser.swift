import Foundation

public struct SSEMessage: Hashable, Sendable {
    public var event: String
    public var data: String
    public var id: String?

    public init(event: String, data: String, id: String? = nil) {
        self.event = event
        self.data = data
        self.id = id
    }
}

public enum SSEFrame: Hashable, Sendable {
    case message(SSEMessage)
    /// A `:` comment line; rocketd sends `: ping` every 15s.
    case comment(String)
    case retry(Int)
}

/// Incremental Server-Sent Events parser (WHATWG event-stream rules).
/// Feed it raw byte chunks of any size; complete frames are returned.
public struct SSEParser: Sendable {
    private var buffer: [UInt8] = []
    private var lastWasCR = false
    private var eventName = ""
    private var dataLines: [String] = []
    private var hasData = false
    private var lastEventID: String?

    public init() {}

    public mutating func feed<S: Sequence>(_ bytes: S) -> [SSEFrame] where S.Element == UInt8 {
        var frames: [SSEFrame] = []
        for byte in bytes {
            switch byte {
            case UInt8(ascii: "\n"):
                if lastWasCR {
                    lastWasCR = false // CRLF: the CR already ended the line
                    continue
                }
                endLine(into: &frames)
            case UInt8(ascii: "\r"):
                lastWasCR = true
                endLine(into: &frames)
            default:
                lastWasCR = false
                buffer.append(byte)
            }
        }
        return frames
    }

    private mutating func endLine(into frames: inout [SSEFrame]) {
        let line = String(decoding: buffer, as: UTF8.self)
        buffer.removeAll(keepingCapacity: true)

        if line.isEmpty {
            if hasData {
                frames.append(.message(SSEMessage(event: eventName.isEmpty ? "message" : eventName,
                                                  data: dataLines.joined(separator: "\n"), id: lastEventID)))
            }
            eventName = ""
            dataLines.removeAll()
            hasData = false
            return
        }
        if line.hasPrefix(":") {
            frames.append(.comment(String(line.dropFirst()).trimmingLeadingSpace()))
            return
        }
        let field: Substring
        var value: Substring = ""
        if let colon = line.firstIndex(of: ":") {
            field = line[..<colon]
            value = line[line.index(after: colon)...]
            if value.first == " " { value = value.dropFirst() }
        } else {
            field = Substring(line)
        }
        switch field {
        case "event":
            eventName = String(value)
        case "data":
            dataLines.append(String(value))
            hasData = true
        case "id":
            if !value.contains("\0") { lastEventID = String(value) }
        case "retry":
            if let ms = Int(value), ms >= 0 { frames.append(.retry(ms)) }
        default:
            break
        }
    }
}

private extension String {
    func trimmingLeadingSpace() -> String {
        first == " " ? String(dropFirst()) : self
    }
}
