import SwiftUI

/// Native empty state that fills a detail pane or the space below its header.
struct DetailEmptyState: View {
    let title: String
    let systemImage: String
    let description: String?

    init(_ title: String, systemImage: String, description: String? = nil) {
        self.title = title
        self.systemImage = systemImage
        self.description = description
    }

    var body: some View {
        ContentUnavailableView {
            Label {
                Text(title).foregroundStyle(Color.xeanInk)
            } icon: {
                Image(systemName: systemImage).foregroundStyle(Color.xeanSecondaryInk)
            }
        } description: {
            if let description {
                Text(description)
                    .foregroundStyle(Color.xeanSecondaryInk)
                    .multilineTextAlignment(.center)
                    .frame(maxWidth: 440)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
