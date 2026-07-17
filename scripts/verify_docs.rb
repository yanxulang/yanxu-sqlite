# frozen_string_literal: true

require "json"
require "pathname"

api_path = Pathname(ARGV.fetch(0)).expand_path
root = Pathname(ARGV.fetch(1, ".")).expand_path

required = %w[
  README.md
  LICENSE
  CHANGELOG.md
  COMPATIBILITY.md
  SECURITY.md
  CONTRIBUTING.md
  docs/API.md
  docs/GUIDE.md
  docs/ARCHITECTURE.md
  docs/MIGRATION.md
  docs/PERFORMANCE.md
  docs/SECURITY_MODEL.md
]

missing_files = required.reject { |path| (root / path).file? && !(root / path).zero? }
abort "缺少文档：#{missing_files.join('、')}" unless missing_files.empty?

api = JSON.parse(api_path.read)
api_document = (root / "docs/API.md").read
names = api.fetch("declarations").flat_map do |declaration|
  [declaration["name"]] + declaration.fetch("methods", []).map { |method| method["name"] }
end.compact.uniq
missing_names = names.reject { |name| api_document.include?(name) }
abort "API 文档缺少：#{missing_names.join('、')}" unless missing_names.empty?

markdown_files = root.glob("**/*.md").reject { |path| path.each_filename.include?("target") }
broken_links = []
markdown_files.each do |file|
  file.read.scan(/\[[^\]]+\]\(([^)]+)\)/).flatten.each do |link|
    next if link.start_with?("http", "#")

    target = file.dirname / link.split("#", 2).first
    broken_links << "#{file.relative_path_from(root)}: #{link}" unless target.exist?
  end
end
abort "失效链接：\n#{broken_links.join("\n")}" unless broken_links.empty?

forbidden_placeholders = ["TODO", "FIXME", "计划完成", "理论支持", "预计成功"]
placeholder_hits = markdown_files.map do |file|
  hit = forbidden_placeholders.find { |placeholder| file.read.include?(placeholder) }
  "#{file.relative_path_from(root)}: #{hit}" if hit
end.compact
abort "文档含占位陈述：\n#{placeholder_hits.join("\n")}" unless placeholder_hits.empty?

puts "文档通过：#{markdown_files.length} 个文件，#{names.length} 个公开名称"
