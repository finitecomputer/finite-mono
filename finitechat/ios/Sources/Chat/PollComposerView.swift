import SwiftUI

struct PollComposerDraft: Identifiable, Equatable {
    let roomID: String

    var id: String { roomID }
}

struct PollComposerView: View {
    @Environment(\.dismiss) private var dismiss

    let onSubmit: (String, [String]) -> Bool

    @State private var question = ""
    @State private var options = [
        PollComposerOption(text: ""),
        PollComposerOption(text: "")
    ]

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField("Question", text: $question, axis: .vertical)
                        .lineLimit(1...4)
                        .textInputAutocapitalization(.sentences)
                        .accessibilityLabel("Poll question")
                }

                Section {
                    ForEach($options) { $option in
                        HStack(spacing: 10) {
                            TextField("Option", text: $option.text, axis: .vertical)
                                .lineLimit(1...3)
                                .textInputAutocapitalization(.sentences)
                                .accessibilityLabel("Poll option")

                            if options.count > minimumPollOptions {
                                Button {
                                    removeOption(option.id)
                                } label: {
                                    Image(systemName: "minus.circle.fill")
                                        .foregroundStyle(.secondary)
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel("Remove option")
                            }
                        }
                    }

                    if options.count < maximumPollOptions {
                        Button {
                            addOption()
                        } label: {
                            Label("Add Option", systemImage: "plus.circle")
                        }
                    }
                }
            }
            .navigationTitle("Poll")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    GlassCircleCloseButton { dismiss() }
                }

                ToolbarItem(placement: .confirmationAction) {
                    Button("Send") {
                        submit()
                    }
                    .disabled(!canSubmit)
                }
            }
        }
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
    }

    private var normalizedQuestion: String {
        question.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var normalizedOptions: [String] {
        options
            .map { $0.text.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
    }

    private var canSubmit: Bool {
        !normalizedQuestion.isEmpty && normalizedOptions.count >= minimumPollOptions
    }

    private func addOption() {
        guard options.count < maximumPollOptions else { return }
        let option = PollComposerOption(text: "")
        options.append(option)
    }

    private func removeOption(_ id: UUID) {
        guard options.count > minimumPollOptions else { return }
        options.removeAll { $0.id == id }
    }

    private func submit() {
        guard canSubmit else { return }
        if onSubmit(normalizedQuestion, normalizedOptions) {
            dismiss()
        }
    }
}

private struct PollComposerOption: Identifiable, Equatable {
    let id = UUID()
    var text: String
}

private let minimumPollOptions = 2
private let maximumPollOptions = 10
