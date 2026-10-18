# std 目録の MD5 は標高画像には使えないため、PNG の内容を検査する。
require 'zlib'
require 'digest'
require 'json'
require 'time'
DEM_RECORD_LOCK = Mutex.new

def valid_dem_png?(body)
  return false unless body.start_with?([137,80,78,71,13,10,26,10].pack('C*'))
  offset=8
  compressed=''.b
  header=false
  ended=false
  while offset+12<=body.bytesize
    length=body.byteslice(offset,4).unpack1('N')
    kind=body.byteslice(offset+4,4)
    payload=body.byteslice(offset+8,length)
    crc=body.byteslice(offset+8+length,4)
    return false unless payload && crc && crc.bytesize==4
    return false unless Zlib.crc32(kind+payload)==crc.unpack1('N')
    if kind=='IHDR'
      return false unless offset==8 && length==13
      return false unless payload.unpack('NNCCCCC')==[256,256,8,2,0,0,0]
      header=true
    elsif kind=='IDAT'
      compressed<<payload
    elsif kind=='IEND'
      ended=true
      break
    end
    offset+=length+12
  end
  return false unless header && ended
  pixels=Zlib::Inflate.inflate(compressed)
  return false unless pixels.bytesize==256*769
  (0...256).all?{|row| pixels.getbyte(row*769)<=4}
rescue StandardError
  false
end

def record_dem_tile(url,path,body,status)
  DEM_RECORD_LOCK.synchronize do
    File.open('dem-downloads.jsonl','a'){|f| f.puts(JSON.generate({url:url,path:path,status:status,catalogSha256:ENV['DEM_CATALOG_SHA256'],checkedAt:Time.now.utc.iso8601,bytes:body&.bytesize,sha256:body && Digest::SHA256.hexdigest(body)})); f.flush; f.fsync}
  end
end

def record_dem_not_found(error,url,path)
  return false unless error.io.respond_to?(:status) && error.io.status[0]=='404'
  record_dem_tile(url,path,nil,404)
  true
end
