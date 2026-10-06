import Foundation
import Testing
@testable import RocketKit

@MainActor
@Suite("Environment metadata and selection")
struct EnvironmentTests {
    private func summary(_ json: String) throws -> Summary {
        try RocketJSON.decoder.decode(Summary.self, from: Data(json.utf8))
    }

    @Test func declaredMetadataSurvivesDecodingAndKeepsEmptyDistinctFromAbsent() throws {
        for json in [
            #"{"name":"p","default_env":"dev","envs":["dev","smoke","stage"]}"#,
            #"{"name":"p","default_env":"dev","envs":[]}"#,
        ] {
            let info = try RocketJSON.decoder.decode(ProjectInfo.self, from: Data(json.utf8))
            let encoded = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(info)) as? [String: Any]
            let original = try JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any]
            #expect(encoded?["envs"] as? [String] == original?["envs"] as? [String])
        }
        let older = try RocketJSON.decoder.decode(ProjectInfo.self, from: Data(#"{"name":"p","default_env":"dev"}"#.utf8))
        let encoded = try JSONSerialization.jsonObject(with: RocketJSON.encoder.encode(older)) as? [String: Any]
        #expect(encoded?["envs"] == nil)
    }

    @Test func declarationsExcludeStaleRunsAndIncludeUnstartedEnvironments() throws {
        let store = RocketStore()
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev","envs":["dev","smoke","stage"]},"services":[{"project":"p","service":"web","env":"removed","state":"stopped"}]}"#), project: "p")
        #expect(store.knownEnvs(for: "p") == ["dev", "smoke", "stage"])
        try store.apply(RocketJSON.decoder.decode(DaemonEvent.self, from: Data(#"{"type":"service.state","run":{"project":"p","service":"legacy","env":"historical","state":"running"}}"#.utf8)))
        #expect(store.knownEnvs(for: "p") == ["dev", "smoke", "stage"])
    }

    @Test func emptyDeclarationDoesNotFallBackToDefaultOrRuns() throws {
        let store = RocketStore()
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev","envs":[]},"services":[{"project":"p","service":"web","env":"smoke","state":"stopped"}]}"#), project: "p")
        #expect(store.knownEnvs(for: "p").isEmpty)
    }

    @Test func olderDaemonUsesDefaultAndSeenEnvironmentsOnlyForItsProject() throws {
        let store = RocketStore()
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev"},"services":[{"project":"p","service":"web","env":"smoke","state":"stopped"}]}"#), project: "p")
        var other = Run(project: "other", service: "web", state: .running)
        other.env = "unrelated"
        store.replaceRuns([other], project: "other")
        #expect(store.knownEnvs(for: "p") == ["dev", "smoke"])
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev","envs":["dev","stage"]}}"#), project: "p")
        #expect(store.knownEnvs(for: "p") == ["dev", "stage"])
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev"},"services":[{"project":"p","service":"web","env":"smoke","state":"running"}]}"#), project: "p")
        #expect(store.knownEnvs(for: "p") == ["dev", "smoke"])
    }

    @Test func removedSelectionResetsAcrossProjectsAndDefaultStaysNil() throws {
        let store = RocketStore()
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev","envs":["dev","smoke"]}}"#), project: "p")
        store.applySummary(try summary(#"{"project":{"name":"other","default_env":"dev","envs":["dev","stage"]}}"#), project: "other")
        let selections = ["p": "smoke", "other": "stage"]
        #expect(store.validatedEnv(nil, for: "p") == nil)
        #expect(store.validatedEnvironmentSelections(selections) == selections)
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev","envs":["dev"]},"services":[{"project":"p","service":"old","env":"smoke","state":"stopped"}]}"#), project: "p")
        #expect(store.validatedEnv("smoke", for: "p") == nil)
        #expect(store.validatedEnv("dev", for: "p") == "dev")
        #expect(store.validatedEnvironmentSelections(selections) == ["other": "stage"])
        store.applySummary(try summary(#"{"project":{"name":"other","envs":[]}}"#), project: "other")
        #expect(store.validatedEnvironmentSelections(selections).isEmpty)
    }

    @Test func selectionValidationUsesOlderDaemonFallback() throws {
        let store = RocketStore()
        store.applySummary(try summary(#"{"project":{"name":"p","default_env":"dev"},"services":[{"project":"p","service":"web","env":"smoke","state":"stopped"}]}"#), project: "p")
        #expect(store.validatedEnv("smoke", for: "p") == "smoke")
        #expect(store.validatedEnv("unknown", for: "p") == nil)
        #expect(store.validatedEnvironmentSelections(["p": "dev", "missing": "dev"]) == ["p": "dev"])
    }
}
