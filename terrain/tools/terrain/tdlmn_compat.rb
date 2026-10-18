# 公式ツールの古い例外名を現在の Ruby に合わせる。公式ソースは変更しない。
require 'timeout'
TimeoutError = Timeout::Error unless defined?(TimeoutError)

# バックグラウンド実行でも進捗をすぐログへ出す。
$stdout.sync = true
$stderr.sync = true
# 取得スレッドの致命的なエラーで処理全体を止め、キュー待ちのまま残さない。
Thread.abort_on_exception = true
