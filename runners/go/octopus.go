// Package octopus is the trusted, versioned host-call API of sagat-inline-v1.
package octopus

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"sync"
	"syscall"
)

type Response struct {
	Status  int               `json:"status"`
	Headers map[string]string `json:"headers"`
	Body    string            `json:"body"`
}

type Client struct {
	channel        io.Writer
	replies        *bufio.Scanner
	mu             sync.Mutex
	next           uint64
	IdempotencyKey string
}

func Channel() (*os.File, error) {
	fd, err := syscall.Dup(1)
	if err != nil {
		return nil, err
	}
	os.Stdout = os.Stderr
	return os.NewFile(uintptr(fd), "sagat-channel"), nil
}

func New(channel io.Writer, replies io.Reader, key string) *Client {
	scanner := bufio.NewScanner(replies)
	scanner.Buffer(make([]byte, 4096), 272*1024)
	return &Client{channel: channel, replies: scanner, IdempotencyKey: key}
}

func (c *Client) call(request map[string]any, reply any) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.next++
	request["id"] = c.next
	bytes, err := json.Marshal(request)
	if err != nil {
		return err
	}
	if _, err = c.channel.Write(append(bytes, '\n')); err != nil {
		return err
	}
	for c.replies.Scan() {
		var envelope map[string]json.RawMessage
		if json.Unmarshal(c.replies.Bytes(), &envelope) != nil {
			continue
		}
		var id uint64
		if json.Unmarshal(envelope["id"], &id) != nil || id != c.next {
			continue
		}
		var ok bool
		if json.Unmarshal(envelope["ok"], &ok) != nil || !ok {
			var message string
			_ = json.Unmarshal(envelope["error"], &message)
			if message == "" {
				message = "refused"
			}
			return errors.New(message)
		}
		return json.Unmarshal(c.replies.Bytes(), reply)
	}
	return errors.New("the platform closed the channel")
}

func (c *Client) Fetch(url, method string, headers map[string]string, body string) (Response, error) {
	if method == "" {
		method = "GET"
	}
	if headers == nil {
		headers = map[string]string{}
	}
	var response Response
	err := c.call(map[string]any{"op": "fetch", "url": url, "method": method, "headers": headers, "body": body}, &response)
	return response, err
}

func (c *Client) Secret(name string) (string, error) {
	var response struct {
		Value string `json:"value"`
	}
	err := c.call(map[string]any{"op": "secret", "name": name}, &response)
	return response.Value, err
}

func (c *Client) Log(values ...any) { fmt.Fprintln(os.Stderr, values...) }

func (c *Client) Result(value any) {
	c.write(map[string]any{"op": "result", "value": value})
}

func (c *Client) Fail(err error) {
	c.write(map[string]any{"op": "error", "message": err.Error(), "stack": ""})
}

func (c *Client) write(value any) {
	bytes, err := json.Marshal(value)
	if err != nil {
		bytes = []byte(`{"op":"error","message":"result is not JSON","stack":""}`)
	}
	_, _ = c.channel.Write(append(bytes, '\n'))
}
